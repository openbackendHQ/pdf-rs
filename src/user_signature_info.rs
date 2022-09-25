use cryptographic_message_syntax::SignerBuilder;
use serde::{Deserialize, Serialize};

/// The info provided to PDF service when a document needs to be signed.
#[derive(Clone)]
pub struct UserSignatureInfo<'a> {
    pub box_id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_email: String,
    pub user_signature: Vec<u8>,
    pub user_signing_keys: SignerBuilder<'a>,
}

/// The info inside the PDF form signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserFormSignatureInfo {
    pub user_id: String,
    pub box_id: String,
}

impl UserFormSignatureInfo {
    pub fn new(user_id: String, box_id: String) -> Self {
        UserFormSignatureInfo { user_id, box_id }
    }
}
