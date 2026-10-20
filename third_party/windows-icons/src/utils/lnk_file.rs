use std::str;
use std::error::Error;
use tokio::process::Command;

pub async fn get_target_lnk_file(
    lnk_path: &str
    ) -> Result<String, Box<dyn Error>> {

    let lnk_target = Command::new("powershell")
        .arg("-Command")
        .arg(format!(
            r#"
                $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut("{}")
                $shortcut.TargetPath
            "#,
            lnk_path
        ))
        .output()
        .await?;

    if lnk_target.status.success() {
        let target_path = str::from_utf8(&lnk_target.stdout)?.trim().to_string();
        Ok(target_path)
    } else {
        let error_message = str::from_utf8(&lnk_target.stderr)?.trim().to_string();
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            error_message,
        )))
    }
}
