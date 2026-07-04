use image::DynamicImage;
use std::io::Cursor;

pub fn load_and_orient(bytes: &[u8]) -> Result<DynamicImage, String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| format!("Failed to load image: {}", e))?;
    
    // Attempt to read EXIF orientation
    let mut reader = Cursor::new(bytes);
    if let Ok(exif_data) = exif::Reader::new().read_from_container(&mut reader) {
        if let Some(field) = exif_data.get_field(exif::Tag::Orientation, exif::In::PRIMARY) {
            if let Some(orientation) = field.value.get_uint(0) {
                img = match orientation {
                    2 => img.fliph(),
                    3 => img.rotate180(),
                    4 => img.flipv(),
                    5 => img.rotate90().fliph(),
                    6 => img.rotate90(),
                    7 => img.rotate270().fliph(),
                    8 => img.rotate270(),
                    _ => img,
                };
            }
        }
    }
    
    Ok(img)
}
