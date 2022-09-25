pub fn parse_font(font_string: Option<&str>) -> ((&str, i32), (&str, i32, i32, i32, i32)) {
  // The default font object (/Helv 12 Tf 0 g)
  let default_font = ("Helv", 12);
  let default_color = ("g", 0, 0, 0, 0);

  // Build the font basing on the default appearance, if exists, if not,
  // assume a default font (surely to be improved!)
  match font_string {
      Some(font_string) => {
          let font = font_string
              .trim_start_matches('/')
              .split("Tf")
              .collect::<Vec<_>>();

          if font.len() < 2 {
              (default_font, default_color)
          } else {
              let font_family = font[0].trim().split(' ').collect::<Vec<_>>();
              let font_color = font[1].trim().split(' ').collect::<Vec<_>>();

              let font = if font_family.len() >= 2 {
                  (font_family[0], font_family[1].parse::<i32>().unwrap_or(0))
              } else {
                  default_font
              };

              let color = if font_color.len() == 2 {
                  ("g", font_color[0].parse::<i32>().unwrap_or(0), 0, 0, 0)
              } else if font_color.len() == 4 {
                  (
                      "rg",
                      font_color[0].parse::<i32>().unwrap_or(0),
                      font_color[1].parse::<i32>().unwrap_or(0),
                      font_color[2].parse::<i32>().unwrap_or(0),
                      0,
                  )
              } else if font_color.len() == 5 {
                  (
                      "k",
                      font_color[0].parse::<i32>().unwrap_or(0),
                      font_color[1].parse::<i32>().unwrap_or(0),
                      font_color[2].parse::<i32>().unwrap_or(0),
                      font_color[3].parse::<i32>().unwrap_or(0),
                  )
              } else {
                  default_color
              };

              (font, color)
          }
      }
      _ => (default_font, default_color),
  }
}
