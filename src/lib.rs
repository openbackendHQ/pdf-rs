mod acro_form;
mod byte_range;
mod digitally_sign;
mod error;
mod image_insert;
mod image_insert_to_page;
mod image_xobject;
mod lopdf_utils;
mod pdf_object;
mod rectangle;
mod signature_image;
mod signature_info;
mod user_signature_info;
mod utils;

use acro_form::AcroForm;
use bitflags::_core::str::from_utf8;
use byte_range::ByteRange;
use image_insert::InsertImage;
use image_insert_to_page::InsertImageToPage;
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, IncrementalDocument, Object, ObjectId, Stream,
};
use pdf_object::PdfObjectDeref;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::{fs::File, path::Path};
use utils::parse_font;

pub use error::Error;
pub use lopdf;
pub use user_signature_info::{UserFormSignatureInfo, UserSignatureInfo};

/// The whole PDF document. This struct only loads part of the document on demand.
#[derive(Debug, Clone)]
pub struct PDFSigningDocument {
    raw_document: IncrementalDocument,
    file_name: String,
    /// Link between the image name saved and the objectId of the image.
    /// This is used to reduce the amount of copies of the images in the pdf file.
    image_signature_object_id: HashMap<String, ObjectId>,

    acro_form: Option<Vec<AcroForm>>,
}

impl PDFSigningDocument {
    fn new(raw_document: IncrementalDocument, file_name: String) -> Self {
        PDFSigningDocument {
            raw_document,
            file_name,
            image_signature_object_id: HashMap::new(),
            acro_form: None,
        }
    }

    pub fn copy_from(&mut self, other: Self) {
        self.raw_document = other.raw_document;
        self.file_name = other.file_name;
        // Do not replace `image_signature_object_id`
        // We want to keep this so we can do optimization.
        self.acro_form = other.acro_form;
    }

    pub fn read_from<R: std::io::Read>(reader: R, file_name: String) -> Result<Self, Error> {
        let raw_doc = IncrementalDocument::load_from(reader)?;
        Ok(Self::new(raw_doc, file_name))
    }

    pub fn read<P: AsRef<Path>>(path: P, file_name: String) -> Result<Self, Error> {
        let raw_doc = IncrementalDocument::load(path)?;
        Ok(Self::new(raw_doc, file_name))
    }

    pub fn load_all(&mut self) -> Result<(), Error> {
        self.load_acro_form()
    }

    pub fn load_acro_form(&mut self) -> Result<(), Error> {
        if self.acro_form.is_none() {
            self.acro_form = Some(AcroForm::load_all_forms(
                self.raw_document.get_prev_documents(),
            )?);
        } else {
            log::info!("Already Loaded Acro Form.");
        }
        Ok(())
    }

    /// Save document to file
    pub fn save_document<P: AsRef<Path>>(&self, path: P) -> Result<File, Error> {
        // Create clone so we can compress the clone, not the original.
        let mut raw_document = self.raw_document.clone();
        raw_document.new_document.compress();
        Ok(raw_document.save(path)?)
    }

    /// Write document to Writer or buffer
    pub fn write_document<W: std::io::Write>(&self, target: &mut W) -> Result<(), Error> {
        // Create clone so we can compress the clone, not the original.
        let mut raw_document = self.raw_document.clone();
        raw_document.new_document.compress();
        raw_document.save_to(target)?;
        Ok(())
    }

    pub fn get_incr_document_ref(&self) -> &IncrementalDocument {
        &self.raw_document
    }

    pub fn get_prev_document_ref(&self) -> &Document {
        self.raw_document.get_prev_documents()
    }

    pub fn get_new_document_ref(&self) -> &Document {
        &self.raw_document.new_document
    }

    pub fn sign_document_2(
        &mut self,
        users_signature_info: Vec<UserSignatureInfo>,
    ) -> Result<Vec<u8>, Error> {
        self.load_all()?;
        // Set PDF version, version 1.5 is the minimum version required.
        self.raw_document.new_document.version = "1.5".to_owned();

        // loop over AcroForm elements
        let acro_forms_opts = self.acro_form.clone();
        let mut last_binary_pdf = None;

        // Covert `Vec<UserSignatureInfo>` to `HashMap<String, UserSignatureInfo>`
        let users_signature_info_map: HashMap<String, UserSignatureInfo> = users_signature_info
            .iter()
            .map(|info| (info.box_id.clone(), info.clone()))
            .collect();

        if acro_forms_opts.is_some() {
            let acro_forms = acro_forms_opts.unwrap();

            for form_field in acro_forms.into_iter() {
                // Check if it is a signature and it is already signed.
                if !form_field.is_empty_signature() {
                    // Go back to start of while loop
                    continue;
                }

                // Update pdf (when nothing else is incorrect)
                // Insert signature images into pdf itself.
                let pdf_document_user_info_opt =
                    self.add_signature_images_2(form_field, &users_signature_info_map)?;

                // PDF has been updated, now we need to digitally sign it.
                if let Some((pdf_document_image, user_form_info)) = pdf_document_user_info_opt {
                    // Digitally sign the document using a cert.
                    let user_info = users_signature_info_map
                        .get(&user_form_info.box_id)
                        .ok_or_else(|| Error::Other("User was not found".to_owned()))?;

                    let new_binary_pdf = pdf_document_image.digitally_sign_document(user_info)?;
                    // Reload file
                    self.copy_from(Self::read_from(
                        &*new_binary_pdf,
                        pdf_document_image.file_name,
                    )?);
                    self.load_all()?;
                    self.raw_document.new_document.version = "1.5".to_owned();

                    // acro_forms = self.acro_form.clone();
                    // Set as return value
                    last_binary_pdf = Some(new_binary_pdf);
                    // Reset form field index
                    // form_field_index = 0;
                }
            }
        }

        match last_binary_pdf {
            Some(last_binary_pdf) => Ok(last_binary_pdf),
            None => {
                // No signing done, so just return initial document.
                Ok(self.raw_document.get_prev_documents_bytes().to_vec())
            }
        }
    }

    pub fn sign_document(
        &mut self,
        users_signature_info: Vec<UserSignatureInfo>,
    ) -> Result<Vec<u8>, Error> {
        self.load_all()?;
        // Set PDF version, version 1.5 is the minimum version required.
        self.raw_document.new_document.version = "1.5".to_owned();

        // loop over AcroForm elements
        let mut acro_forms = self.acro_form.clone();
        let mut last_binary_pdf = None;

        // Take the first form field (if there is any)
        let mut form_field_current = acro_forms.as_ref().and_then(|list| list.first().cloned());
        let mut form_field_index = 0;

        // Covert `Vec<UserSignatureInfo>` to `HashMap<String, UserSignatureInfo>`
        let users_signature_info_map: HashMap<String, UserSignatureInfo> = users_signature_info
            .iter()
            .map(|info| (info.user_id.clone(), info.clone()))
            .collect();

        // Make sure we never end up in an infinite loop, should not happen.
        // But better safe then sorry.
        let mut loop_counter: u16 = 0;
        // Loop over all the form fields and sign them one by one.
        while let Some(form_field) = form_field_current {
            loop_counter += 1;
            if loop_counter >= 10000 {
                log::error!(
                    "Infinite loop detected and prevented. Please check file: `{}`.",
                    self.file_name
                );
                break;
            }
            // Check if it is a signature and it is already signed.
            if !form_field.is_empty_signature() {
                // Go to next form field if pdf did not change
                form_field_index += 1;
                form_field_current = acro_forms
                    .as_ref()
                    .and_then(|list| list.get(form_field_index).cloned());
                // Go back to start of while loop
                continue;
            }

            // TODO: Debug code, can be removed
            // if form_field_index == 1 {
            //     form_field_index += 1;
            //     form_field_current = acro_forms
            //         .as_ref()
            //         .and_then(|list| list.get(form_field_index).cloned());
            //     continue;
            // }

            // Update pdf (when nothing else is incorrect)
            // Insert signature images into pdf itself.
            let pdf_document_user_info_opt =
                self.add_signature_images(form_field, &users_signature_info_map)?;

            // PDF has been updated, now we need to digitally sign it.
            if let Some((pdf_document_image, user_form_info)) = pdf_document_user_info_opt {
                // Digitally sign the document using a cert.
                let user_info = users_signature_info_map
                    .get(&user_form_info.user_id)
                    .ok_or_else(|| Error::Other("User was not found".to_owned()))?;

                let new_binary_pdf = pdf_document_image.digitally_sign_document(user_info)?;
                // Reload file
                self.copy_from(Self::read_from(
                    &*new_binary_pdf,
                    pdf_document_image.file_name,
                )?);
                self.load_all()?;
                self.raw_document.new_document.version = "1.5".to_owned();
                acro_forms = self.acro_form.clone();
                // Set as return value
                last_binary_pdf = Some(new_binary_pdf);
                // Reset form field index
                form_field_index = 0;
            } else {
                // Go to next form field because pdf did not change
                form_field_index += 1;
            }

            // Load next form field (or set to `0` depending on index.)
            form_field_current = acro_forms
                .as_ref()
                .and_then(|list| list.get(form_field_index).cloned());
        }

        match last_binary_pdf {
            Some(last_binary_pdf) => Ok(last_binary_pdf),
            None => {
                // No signing done, so just return initial document.
                Ok(self.raw_document.get_prev_documents_bytes().to_vec())
            }
        }
    }

    // pub fn add_signature_to_form<R: Read>(
    //     &mut self,
    //     image_reader: R,
    //     image_name: &str,
    //     page_id: ObjectId,
    //     form_id: ObjectId,
    // ) -> Result<ObjectId, Error> {
    //     let rect = Rectangle::get_rectangle_from_signature(form_id, &self.raw_document)?;
    //     let image_object_id_opt = self.image_signature_object_id.get(image_name).cloned();
    //     Ok(if let Some(image_object_id) = image_object_id_opt {
    //         // Image was already added so we can reuse it.
    //         self.add_image_to_page_only(image_object_id, image_name, page_id, rect)?
    //     } else {
    //         // Image was not added already so we need to add it in full
    //         let image_object_id = self.add_image(image_reader, image_name, page_id, rect)?;
    //         // Add signature to map
    //         self.image_signature_object_id
    //             .insert(image_name.to_owned(), image_object_id);
    //         image_object_id
    //     })
    // }

    pub fn fill_form(&mut self, data: Map<String, Value>) -> Result<(), Error> {
        let mut doc = self.raw_document.get_prev_documents().clone();

        // inspired by https://github.com/Emulator000/pdf_form/blob/master/src/lib.rs

        let acro_forms = self.acro_form.clone();
        let form_fields_opts = acro_forms.as_ref();
        if form_fields_opts.is_some() {
            let form_fields = form_fields_opts.unwrap();
            for field in form_fields.iter() {
                let object_id_opts = field.get_object_id();
                let partial_field_name = field.get_partial_field_name().unwrap_or("");
                let partial_field_name_lower_case = partial_field_name.to_lowercase();

                let data_value_opts = data.get(&partial_field_name_lower_case);
                if data_value_opts.is_some() && object_id_opts.is_some() {
                    let object_id = object_id_opts.unwrap();
                    let data_value = data_value_opts.unwrap().as_str().unwrap().to_string();

                    let field = doc
                        .get_object_mut(object_id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap();

                    field.set("V", Object::string_literal(data_value.into_bytes()));

                    // ////////
                    // regenerate_text_appearance

                    // The value of the object (should be a string)
                    let value = field.get(b"V")?.to_owned();

                    // The default appearance of the object (should be a string)
                    let da = field.get(b"DA")?.to_owned();

                    // The default appearance of the object (should be a string)
                    let rect = field
                        .get(b"Rect")?
                        .as_array()?
                        .iter()
                        .map(|object| {
                            object
                                .as_f64()
                                .unwrap_or(object.as_i64().unwrap_or(0) as f64)
                                as f32
                        })
                        .collect::<Vec<_>>();

                    // Gets the object stream
                    let object_id = if field.has(b"AP") {
                        let object_id = field.get(b"AP")?.as_dict()?.get(b"N")?.as_reference()?;
                        object_id
                    } else {
                        let new_obj_id = doc.add_object(Object::Stream(Stream::new(
                            dictionary! {},
                            "stream".as_bytes().to_vec(),
                        )));

                        let field = doc
                            .get_object_mut(object_id)
                            .unwrap()
                            .as_dict_mut()
                            .unwrap();

                        field.set(
                            "AP",
                            dictionary! {
                                "N" => Object::Reference(new_obj_id)
                            },
                        );

                        let object_id = field.get(b"AP")?.as_dict()?.get(b"N")?.as_reference()?;

                        object_id
                    };

                    // let object_id = field.get(b"AP")?.as_dict()?.get(b"N")?.as_reference()?;
                    let stream = doc.get_object_mut(object_id)?.as_stream_mut()?;

                    // Decode and get the content, even if is compressed
                    let mut content = {
                        if let Ok(content) = stream.decompressed_content() {
                            Content::decode(&content)?
                        } else {
                            Content::decode(&stream.content)?
                        }
                    };

                    // Ignored operators
                    let ignored_operators = vec![
                        "bt", "tc", "tw", "tz", "g", "tm", "tr", "tf", "tj", "et", "q", "bmc",
                        "emc",
                    ];

                    // Remove these ignored operators as we have to generate the text and fonts again
                    content.operations.retain(|operation| {
                        !ignored_operators.contains(&operation.operator.to_lowercase().as_str())
                    });

                    // Let's construct the text widget
                    content.operations.append(&mut vec![
                        Operation::new("BMC", vec!["Tx".into()]),
                        Operation::new("q", vec![]),
                        Operation::new("BT", vec![]),
                    ]);

                    let font = parse_font(match da {
                        Object::String(ref bytes, _) => Some(from_utf8(bytes)?),
                        _ => None,
                    });

                    // Define some helping font variables
                    let font_name = (font.0).0;
                    let font_size = (font.0).1;
                    let font_color = font.1;

                    // Set the font type and size and color
                    content.operations.append(&mut vec![
                        Operation::new("Tf", vec![font_name.into(), font_size.into()]),
                        Operation::new(
                            font_color.0,
                            match font_color.0 {
                                "k" => vec![
                                    font_color.1.into(),
                                    font_color.2.into(),
                                    font_color.3.into(),
                                    font_color.4.into(),
                                ],
                                "rg" => vec![
                                    font_color.1.into(),
                                    font_color.2.into(),
                                    font_color.3.into(),
                                ],
                                _ => vec![font_color.1.into()],
                            },
                        ),
                    ]);

                    // Calculate the text offset
                    let x = 2.0; // Suppose this fixed offset as we should have known the border here

                    // Formula picked up from Poppler
                    let dy = rect[1] - rect[3];
                    let y = if dy > 0.0 {
                        0.5 * dy - 0.4 * font_size as f32
                    } else {
                        0.5 * font_size as f32
                    };

                    // Set the text bounds, first are fixed at "1 0 0 1" and then the calculated x,y
                    content.operations.append(&mut vec![Operation::new(
                        "Tm",
                        vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
                    )]);

                    // Set the text value and some finalizing operations
                    content.operations.append(&mut vec![
                        Operation::new("Tj", vec![value]),
                        Operation::new("ET", vec![]),
                        Operation::new("Q", vec![]),
                        Operation::new("EMC", vec![]),
                    ]);

                    // Set the new content to the original stream and compress it
                    if let Ok(encoded_content) = content.encode() {
                        stream.set_plain_content(encoded_content);
                        let _ = stream.compress();
                    }
                    // ///////
                }
            }
        }

        // Regenerate the pdf file
        let mut new_binary_pdf: Vec<u8> = Vec::new();
        doc.compress();
        doc.save_to(&mut new_binary_pdf)?;

        self.copy_from(Self::read_from(&*new_binary_pdf, self.file_name.clone())?);
        self.load_all()?;

        Ok(())
    }
}

impl InsertImage for PDFSigningDocument {
    fn add_object<T: Into<Object>>(&mut self, object: T) -> ObjectId {
        self.raw_document.new_document.add_object(object)
    }
}

impl InsertImageToPage for PDFSigningDocument {
    fn add_xobject<N: Into<Vec<u8>>>(
        &mut self,
        page_id: ObjectId,
        xobject_name: N,
        xobject_id: ObjectId,
    ) -> Result<(), Error> {
        Ok(self
            .raw_document
            .add_xobject(page_id, xobject_name, xobject_id)?)
    }

    fn opt_clone_object_to_new_document(&mut self, object_id: ObjectId) -> Result<(), Error> {
        Ok(self
            .raw_document
            .opt_clone_object_to_new_document(object_id)?)
    }

    fn add_to_page_content(
        &mut self,
        page_id: ObjectId,
        content: Content<Vec<Operation>>,
    ) -> Result<(), Error> {
        Ok(self
            .raw_document
            .new_document
            .add_to_page_content(page_id, content)?)
    }
}
