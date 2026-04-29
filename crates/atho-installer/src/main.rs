use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use eframe::egui;
use tempfile::TempDir;
use zip::ZipArchive;

const PAYLOAD_FOOTER_MAGIC: &[u8; 8] = b"ATHOPLD1";
const PAYLOAD_FOOTER_SIZE: u64 = 16;

fn main() -> eframe::Result<()> {
    let bundle = match locate_release_root() {
        Ok(bundle) => bundle,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Atho Setup",
        options,
        Box::new(move |_cc| Box::new(InstallerApp::new(bundle))),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Macos => "macOS",
            Self::Linux => "Linux",
        }
    }

    fn default_install_dir(self) -> String {
        match self {
            Self::Windows => {
                let base = env::var_os("LOCALAPPDATA")
                    .or_else(|| {
                        env::var_os("USERPROFILE").map(|value| {
                            let mut path = PathBuf::from(value);
                            path.push("AppData");
                            path.push("Local");
                            path.into_os_string()
                        })
                    })
                    .unwrap_or_else(|| OsString::from(r"C:\Users\Public\AppData\Local"));
                let mut path = PathBuf::from(base);
                path.push("Programs");
                path.push("Atho");
                path.to_string_lossy().into_owned()
            }
            Self::Macos => home_dir()
                .join("Applications")
                .join("Atho")
                .to_string_lossy()
                .into_owned(),
            Self::Linux => home_dir()
                .join(".local")
                .join("share")
                .join("Atho")
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn default_bin_dir(self) -> String {
        match self {
            Self::Windows => self.default_install_dir(),
            Self::Macos => home_dir().join("bin").to_string_lossy().into_owned(),
            Self::Linux => home_dir()
                .join(".local")
                .join("bin")
                .to_string_lossy()
                .into_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Welcome,
    Finished,
}

struct BundleLocation {
    root: PathBuf,
    _tempdir: Option<TempDir>,
}

impl BundleLocation {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            _tempdir: None,
        }
    }

    fn extracted(root: PathBuf, tempdir: TempDir) -> Self {
        Self {
            root,
            _tempdir: Some(tempdir),
        }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

struct InstallerApp {
    platform: Platform,
    bundle: BundleLocation,
    install_dir: String,
    bin_dir: String,
    launch_after_install: bool,
    screen: Screen,
    log: String,
    error: Option<String>,
}

impl InstallerApp {
    fn new(bundle: BundleLocation) -> Self {
        let platform = Platform::current();
        Self {
            platform,
            bundle,
            install_dir: platform.default_install_dir(),
            bin_dir: platform.default_bin_dir(),
            launch_after_install: true,
            screen: Screen::Welcome,
            log: String::new(),
            error: None,
        }
    }

    fn install(&mut self) -> Result<(), String> {
        self.log.clear();
        self.error = None;

        let output = if cfg!(target_os = "windows") {
            let script = self.bundle.root().join("install.ps1");
            if !script.exists() {
                return Err(format!("missing installer script: {}", script.display()));
            }
            let shell = if command_exists("pwsh") {
                "pwsh"
            } else if command_exists("powershell") {
                "powershell"
            } else {
                return Err("PowerShell is required to install Atho on Windows".to_string());
            };
            Command::new(shell)
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&script)
                .env("ATHO_INSTALL_DIR", &self.install_dir)
                .current_dir(self.bundle.root())
                .output()
                .map_err(|err| format!("failed to run installer: {err}"))?
        } else {
            let script = self.bundle.root().join("install.sh");
            if !script.exists() {
                return Err(format!("missing installer script: {}", script.display()));
            }
            Command::new("/bin/bash")
                .arg(&script)
                .env("ATHO_INSTALL_DIR", &self.install_dir)
                .env("ATHO_BIN_DIR", &self.bin_dir)
                .current_dir(self.bundle.root())
                .output()
                .map_err(|err| format!("failed to run installer: {err}"))?
        };

        self.log.push_str(&String::from_utf8_lossy(&output.stdout));
        self.log.push_str(&String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            return Err(if self.log.is_empty() {
                "installer exited with a failure".to_string()
            } else {
                self.log.clone()
            });
        }

        self.screen = Screen::Finished;

        if self.launch_after_install {
            if let Err(err) = self.launch_installed_client() {
                self.error = Some(err);
            }
        }

        Ok(())
    }

    fn launch_installed_client(&self) -> Result<(), String> {
        let launcher = if cfg!(target_os = "windows") {
            PathBuf::from(&self.install_dir).join("atho.cmd")
        } else {
            PathBuf::from(&self.bin_dir).join("atho")
        };

        if !launcher.exists() {
            return Err(format!(
                "launcher not found after install: {}",
                launcher.display()
            ));
        }

        #[cfg(target_os = "windows")]
        {
            let launcher_string = launcher.to_string_lossy().into_owned();
            let quoted = format!(r#""{}""#, launcher_string);
            Command::new("cmd")
                .args(["/C", quoted.as_str()])
                .spawn()
                .map_err(|err| format!("failed to launch Atho: {err}"))?;
        }

        #[cfg(not(target_os = "windows"))]
        {
            Command::new(&launcher)
                .spawn()
                .map_err(|err| format!("failed to launch Atho: {err}"))?;
        }

        Ok(())
    }

    fn banner(&self) -> &'static str {
        "Atho downloads a release bundle, installs it into a normal user location, and creates simple launchers."
    }
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Atho Setup");
                ui.label(self.banner());
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                ui.label("Platform:");
                ui.monospace(self.platform.label());
                ui.separator();
                ui.label("Release bundle:");
                ui.monospace(self.bundle.root().display().to_string());
            });

            ui.add_space(8.0);

            match self.screen {
                Screen::Welcome => {
                    ui.label("Install location");
                    ui.text_edit_singleline(&mut self.install_dir);
                    ui.add_space(6.0);
                    if self.platform != Platform::Windows {
                        ui.label(format!("Commands will be linked in {}", self.bin_dir));
                    } else {
                        ui.label("The installer will create a Start Menu shortcut and add Atho to your user PATH.");
                    }

                    ui.add_space(10.0);
                    ui.checkbox(&mut self.launch_after_install, "Launch Atho after install");
                    ui.add_space(10.0);

                    if ui.button("Install Atho").clicked() {
                        match self.install() {
                            Ok(()) => {
                                self.error = None;
                            }
                            Err(err) => {
                                self.screen = Screen::Welcome;
                                self.error = Some(err);
                            }
                        }
                    }

                    if let Some(error) = &self.error {
                        ui.add_space(10.0);
                        ui.colored_label(egui::Color32::RED, error);
                    }

                    if !self.log.is_empty() {
                        ui.add_space(10.0);
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                ui.monospace(&self.log);
                            });
                    }
                }
                Screen::Finished => {
                    ui.label("Installation complete.");
                    ui.add_space(8.0);
                    ui.label("You can now launch Atho from the shortcut or command line.");
                    if let Some(error) = &self.error {
                        ui.add_space(8.0);
                        ui.colored_label(egui::Color32::YELLOW, error);
                    }
                    ui.add_space(12.0);
                    if ui.button("Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        });
    }
}

fn command_exists(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn locate_release_root() -> Result<BundleLocation, String> {
    let exe =
        env::current_exe().map_err(|err| format!("failed to locate installer binary: {err}"))?;
    if let Some(bundle) = locate_embedded_bundle(&exe)? {
        return Ok(bundle);
    }

    if cfg!(target_os = "macos") {
        if let Some(app_root) = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            return Ok(BundleLocation::new(app_root.to_path_buf()));
        }
    }

    exe.parent()
        .map(Path::to_path_buf)
        .map(BundleLocation::new)
        .ok_or_else(|| "failed to determine release root".to_string())
}

fn locate_embedded_bundle(exe: &Path) -> Result<Option<BundleLocation>, String> {
    if cfg!(target_os = "macos") {
        if let Some(bundle) = locate_macos_resource_bundle(exe)? {
            return Ok(Some(bundle));
        }
    }

    if cfg!(target_os = "windows") {
        if let Some(bundle) = locate_windows_embedded_bundle(exe)? {
            return Ok(Some(bundle));
        }
    }

    Ok(None)
}

fn locate_macos_resource_bundle(exe: &Path) -> Result<Option<BundleLocation>, String> {
    let Some(contents_root) = exe.parent().and_then(Path::parent) else {
        return Ok(None);
    };
    let payload = contents_root.join("Resources").join("payload.zip");
    if !payload.exists() {
        return Ok(None);
    }
    extract_payload_zip(&payload).map(Some)
}

fn locate_windows_embedded_bundle(exe: &Path) -> Result<Option<BundleLocation>, String> {
    let metadata = match fs::metadata(exe) {
        Ok(metadata) => metadata,
        Err(err) => return Err(format!("failed to inspect installer binary: {err}")),
    };
    if metadata.len() < PAYLOAD_FOOTER_SIZE {
        return Ok(None);
    }

    let mut file =
        File::open(exe).map_err(|err| format!("failed to open installer binary: {err}"))?;
    file.seek(SeekFrom::End(-(PAYLOAD_FOOTER_SIZE as i64)))
        .map_err(|err| format!("failed to read installer footer: {err}"))?;

    let mut footer = [0u8; PAYLOAD_FOOTER_SIZE as usize];
    file.read_exact(&mut footer)
        .map_err(|err| format!("failed to read installer footer: {err}"))?;

    if &footer[..PAYLOAD_FOOTER_MAGIC.len()] != PAYLOAD_FOOTER_MAGIC {
        return Ok(None);
    }

    let payload_len = u64::from_le_bytes(
        footer[PAYLOAD_FOOTER_MAGIC.len()..]
            .try_into()
            .expect("payload footer length"),
    );
    if payload_len > metadata.len().saturating_sub(PAYLOAD_FOOTER_SIZE) {
        return Err("embedded installer payload is truncated".to_string());
    }

    let payload_total = payload_len
        .checked_add(PAYLOAD_FOOTER_SIZE)
        .ok_or_else(|| "embedded installer payload length overflow".to_string())?;
    let payload_start = metadata
        .len()
        .checked_sub(payload_total)
        .ok_or_else(|| "embedded installer payload length underflow".to_string())?;
    file.seek(SeekFrom::Start(payload_start))
        .map_err(|err| format!("failed to seek installer payload: {err}"))?;

    let mut payload = vec![0u8; payload_len as usize];
    file.read_exact(&mut payload)
        .map_err(|err| format!("failed to read embedded installer payload: {err}"))?;

    extract_payload_bytes(&payload)
        .map(Some)
        .map_err(|err| format!("failed to extract embedded installer payload: {err}"))
}

fn extract_payload_zip(path: &Path) -> Result<BundleLocation, String> {
    let bytes = fs::read(path)
        .map_err(|err| format!("failed to read embedded installer payload: {err}"))?;
    extract_payload_bytes(&bytes)
}

fn extract_payload_bytes(bytes: &[u8]) -> Result<BundleLocation, String> {
    let tempdir =
        TempDir::new().map_err(|err| format!("failed to create temporary bundle: {err}"))?;
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|err| format!("invalid installer payload archive: {err}"))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("failed to read installer payload entry: {err}"))?;
        let Some(relative) = entry.enclosed_name().map(PathBuf::from) else {
            return Err("installer payload contains an unsafe path".to_string());
        };
        let outpath = tempdir.path().join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&outpath)
                .map_err(|err| format!("failed to create payload directory: {err}"))?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create payload parent directory: {err}"))?;
        }

        let mut outfile =
            File::create(&outpath).map_err(|err| format!("failed to write payload file: {err}"))?;
        std::io::copy(&mut entry, &mut outfile)
            .map_err(|err| format!("failed to extract payload file: {err}"))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&outpath, fs::Permissions::from_mode(mode & 0o777))
                .map_err(|err| format!("failed to set payload permissions: {err}"))?;
        }
    }

    let root = tempdir.path().to_path_buf();
    Ok(BundleLocation::extracted(root, tempdir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::FileOptions;
    use zip::CompressionMethod;
    use zip::ZipWriter;

    #[test]
    fn payload_zip_extracts_into_temporary_bundle() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut buffer);
            let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
            writer
                .start_file("install.sh", options)
                .expect("start install script");
            writer
                .write_all(b"#!/usr/bin/env bash\nexit 0\n")
                .expect("write install script");
            writer
                .add_directory("nested/", options)
                .expect("start nested directory");
            writer
                .start_file("nested/data.txt", options)
                .expect("start nested file");
            writer.write_all(b"payload").expect("write nested file");
            writer.finish().expect("finish payload");
        }

        let bundle = extract_payload_bytes(buffer.get_ref()).expect("extract payload");
        assert!(bundle.root().join("install.sh").exists());
        assert!(bundle.root().join("nested/data.txt").exists());
    }
}
