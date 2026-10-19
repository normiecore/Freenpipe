use std::{error::Error, fs, path::Path};
use image::RgbaImage;
use crate::utils::image_utils::{get_icon_from_base64, read_image_to_base64};

pub async fn get_uwp_icon(process_path: &str) -> Result<RgbaImage, Box<dyn Error>> {
    let icon_path = get_icon_file_path(process_path).await?;
    let base64 = read_image_to_base64(&icon_path)?;
    let icon = get_icon_from_base64(&base64)?;
    Ok(icon)
}

pub async fn get_uwp_icon_base64(process_path: &str) -> Result<String, Box<dyn Error>> {
    let icon_path = get_icon_file_path(process_path).await?;
    let base64 = read_image_to_base64(&icon_path)?;
    Ok(base64)
}

async fn get_icon_file_path(app_path: &str) -> Result<String, Box<dyn Error>> {
    let package_folder = Path::new(app_path)
        .ancestors()
        .find(|path| path.parent().map_or(false, |p| p.ends_with("WindowsApps")))
        .unwrap();
    
    let manifest_path = package_folder.join("AppxManifest.xml");
    let manifest_content = fs::read_to_string(&manifest_path)?;

    let icon_full_path = extract_icon_path(&manifest_content, &package_folder).await?;

    return Ok(icon_full_path.to_string());
}

async fn extract_icon_path(
    manifest_content: &str, package_folder: &Path
) -> Result<String, Box<dyn Error>> {
    use regex::Regex;

    let re = Regex::new(r#"Square150x150Logo="([^"]+)""#).unwrap();
    let start_tag = "<uap:VisualElements";
    let end_tag = "</uap:VisualElements>";

    if let Some(start) = manifest_content.find(start_tag) {
        if let Some(end) = manifest_content.find(end_tag) {

            let visual_elements_content = &manifest_content[start..end + end_tag.len()];
            if let Some(captures) = re.captures(visual_elements_content) {

                if let Some(icon_relative_path) = captures.get(1) {
                    let icon_relative_path = Path::new(icon_relative_path.as_str());

                    if let Some(icon_file) = icon_relative_path.file_name() {
                        if let Some(icon_file_str) = icon_file.to_str() {

                            let icon_dir_rel = icon_relative_path.parent().unwrap();
                            let icon_search_string = icon_file_str.split('.').next().unwrap_or(icon_file_str);
                            let icon_dir_full_path = package_folder.join(icon_dir_rel);

                            let search_cmd_output = tokio::process::Command::new("powershell")
                                .arg("-Command")
                                .arg(format!(
                                    r#"
                                        Get-ChildItem -Path "{}" -Filter "{}*.png" | ForEach-Object {{ $_.FullName }}
                                    "#,
                                    icon_dir_full_path.to_string_lossy(),
                                    icon_search_string
                                ))
                                .output()
                                .await
                                .expect("failed to run powershell command");
                            
                            let search_output = String::from_utf8_lossy(&search_cmd_output.stdout);
                            let search_results: Vec<&str> = search_output.lines().collect();
                            
                            if let Some(first_result) = search_results.first() {
                                return Ok(first_result.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "Icon path not found in manifest.",
    )))
}
