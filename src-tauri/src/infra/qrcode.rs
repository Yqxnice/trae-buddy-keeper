use anyhow::{anyhow, Result};
use base64::Engine;
use image::Luma;
use qrcode::QrCode;

pub fn generate_qr_base64(text: &str) -> Result<String> {
    let code = QrCode::new(text.as_bytes()).map_err(|e| anyhow!("QR 生成失败: {}", e))?;
    let size = 256u32;
    let img = code.render::<Luma<u8>>().build();
    // resize to size
    let scaled = image::imageops::resize(&img, size, size, image::imageops::FilterType::Nearest);
    let mut buf = Vec::new();
    scaled
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| anyhow!("PNG 编码失败: {}", e))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:image/png;base64,{}", b64))
}
