use std::error::Error;
use base64::engine::general_purpose;
use base64::Engine as _;
use image::RgbaImage;
use utils::image_utils::{get_hicon, icon_to_image};
use utils::process_utils::get_process_path;
use uwp_apps::{get_uwp_icon, get_uwp_icon_base64};
use utils::lnk_file::get_target_lnk_file;

mod utils {
    pub mod image_utils;
    pub mod process_utils;
    pub mod lnk_file;
}
mod uwp_apps;

pub async fn get_icon_by_process_id(process_id: u32) -> Result<RgbaImage, Box<dyn Error>> {
    let path = get_process_path(process_id)?;
    if path.contains("WindowsApps") {
        get_uwp_icon(&path).await
    } else {
        get_icon_by_path(&path)
    }
}

pub fn get_icon_by_path(path: &str) -> Result<RgbaImage, Box<dyn Error>> {
    unsafe {
        match get_hicon(path) {
            Ok(icon) => icon_to_image(icon),
            Err(e) => Err(e),
        }
    }
}

pub async fn get_icon_base64_by_process_id(process_id: u32) -> Result<String, Box<dyn Error>> {
    let path = get_process_path(process_id)?;
    get_icon_base64_by_path(&path).await
}

pub async fn get_icon_base64_by_path(path: &str) -> Result<String, Box<dyn Error>> {
    // this is for when windows apps exe doesn't have icon
    // if path.contains("WindowsApps") {
    //     return get_uwp_icon_base64(path).await;
    // }
    let resolved_path = if path.ends_with(".lnk") {
        match get_target_lnk_file(path).await {
            Ok(target_path) => target_path,
            Err(_) => path.to_string(),
        }
    } else {
        path.to_string()
    };

    let icon_image = get_icon_by_path(&resolved_path)?;
    let mut buffer = Vec::new();
    icon_image.write_to(
        &mut std::io::Cursor::new(&mut buffer),
        image::ImageFormat::Png,
    )?;
    Ok(general_purpose::STANDARD.encode(buffer))
}
