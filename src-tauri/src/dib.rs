pub fn generate_dibv5(img: &image::DynamicImage) -> Vec<u8> {
    let rgba = img.to_rgba8();
    let width = rgba.width() as u32;
    let height = rgba.height() as u32;
    
    // CF_DIBV5 requires BGRA format
    let mut bgra = rgba.into_raw();
    for chunk in bgra.chunks_exact_mut(4) {
        let r = chunk[0];
        let b = chunk[2];
        chunk[0] = b;
        chunk[2] = r;
    }
    
    // Bottom-up pixel arrangement is safest for Windows
    let row_len = (width * 4) as usize;
    let mut bottom_up = Vec::with_capacity(bgra.len());
    for row in bgra.chunks_exact(row_len).rev() {
        bottom_up.extend_from_slice(row);
    }
    
    let mut dibv5 = Vec::with_capacity(124 + bottom_up.len());
    
    // BITMAPV5HEADER
    dibv5.extend_from_slice(&124u32.to_le_bytes()); // bV5Size
    dibv5.extend_from_slice(&(width as i32).to_le_bytes()); // bV5Width
    dibv5.extend_from_slice(&(height as i32).to_le_bytes()); // bV5Height (positive = bottom up)
    dibv5.extend_from_slice(&1u16.to_le_bytes()); // bV5Planes
    dibv5.extend_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    dibv5.extend_from_slice(&3u32.to_le_bytes()); // bV5Compression = BI_BITFIELDS
    dibv5.extend_from_slice(&(bottom_up.len() as u32).to_le_bytes()); // bV5SizeImage
    dibv5.extend_from_slice(&0i32.to_le_bytes()); // bV5XPelsPerMeter
    dibv5.extend_from_slice(&0i32.to_le_bytes()); // bV5YPelsPerMeter
    dibv5.extend_from_slice(&0u32.to_le_bytes()); // bV5ClrUsed
    dibv5.extend_from_slice(&0u32.to_le_bytes()); // bV5ClrImportant
    
    // Color masks for BI_BITFIELDS (BGRA)
    dibv5.extend_from_slice(&0x00FF0000u32.to_le_bytes()); // bV5RedMask
    dibv5.extend_from_slice(&0x0000FF00u32.to_le_bytes()); // bV5GreenMask
    dibv5.extend_from_slice(&0x000000FFu32.to_le_bytes()); // bV5BlueMask
    dibv5.extend_from_slice(&0xFF000000u32.to_le_bytes()); // bV5AlphaMask
    
    // bV5CSType = LCS_sRGB
    dibv5.extend_from_slice(&0x73524742u32.to_le_bytes()); 
    
    // endpoints (36 bytes)
    dibv5.extend_from_slice(&[0u8; 36]);
    
    // Gamma
    dibv5.extend_from_slice(&0u32.to_le_bytes()); // bV5GammaRed
    dibv5.extend_from_slice(&0u32.to_le_bytes()); // bV5GammaGreen
    dibv5.extend_from_slice(&0u32.to_le_bytes()); // bV5GammaBlue
    
    // Intent = LCS_GM_IMAGES
    dibv5.extend_from_slice(&4u32.to_le_bytes());
    
    // Profile data/size, reserved
    dibv5.extend_from_slice(&0u32.to_le_bytes());
    dibv5.extend_from_slice(&0u32.to_le_bytes());
    dibv5.extend_from_slice(&0u32.to_le_bytes());
    
    // Append pixel data
    dibv5.extend_from_slice(&bottom_up);
    
    dibv5
}
