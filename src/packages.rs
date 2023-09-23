use anyhow::Context;
use camino::Utf8PathBuf;
use fs_err as fs;
use std::io;
use std::io::BufWriter;
use tempfile::NamedTempFile;
use tracing::info;

pub fn download_wheel_cached(filename: &str, url: &str) -> anyhow::Result<Utf8PathBuf> {
    let wheels_cache = crate::crate_cache_dir()?.join("wheels");
    let cached_wheel = wheels_cache.join(filename);
    if cached_wheel.is_file() {
        info!("Using cached wheel at {cached_wheel}");
        return Ok(cached_wheel);
    }

    info!("Downloading wheel from {url} to {cached_wheel}");
    fs::create_dir_all(&wheels_cache)?;
    let mut tempfile = NamedTempFile::new_in(wheels_cache)?;
    let tempfile_path = tempfile.path().to_path_buf();
    let mut response = minreq::get(url).send_lazy()?;
    io::copy(&mut response, &mut BufWriter::new(&mut tempfile)).with_context(|| {
        format!(
            "Failed to download wheel from {} to {}",
            url,
            tempfile_path.display()
        )
    })?;
    tempfile
        .persist(&cached_wheel)
        .with_context(|| format!("Failed to persist tempfile to {}", cached_wheel))?;
    Ok(cached_wheel)
}

/// Install wheel, pip and setuptools from the cache
#[cfg(not(feature = "install"))]
fn install_base_packages(
    bin_dir: &Utf8Path,
    venv_python: &Utf8Path,
    site_packages: &Utf8Path,
) -> anyhow::Result<()> {
    // Install packages
    // TODO: Implement our own logic:
    //  * Our own cache and logic to detect whether a wheel is present
    //  * Check if the version is recent (e.g. update if older than 1 month)
    //  * Query pypi API if no, parse versions (pep440) and their metadata
    //  * Download compatible wheel (py3-none-any should do)
    //  * Install into the cache directory
    let prefix = "virtualenv/wheel/3.11/image/1/CopyPipInstall/";
    let wheel_tag = "py3-none-any";
    let packages = &[
        ("pip", "23.2.1"),
        ("setuptools", "68.2.2"),
        ("wheel", "0.41.2"),
    ];
    let virtualenv_data_dir = data_dir()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .context("Couldn't get data dir")?;
    for (name, version) in packages {
        // TODO: acquire lock
        let unpacked_wheel = virtualenv_data_dir
            .join(prefix)
            .join(format!("{name}-{version}-{wheel_tag}"));
        debug!("Installing {name} by copying from {unpacked_wheel}");
        bare::copy_dir_all(&unpacked_wheel, site_packages.as_std_path())
            .with_context(|| format!("Failed to copy {unpacked_wheel} to {site_packages}"))?;

        // Generate launcher
        // virtualenv for some reason creates extra entrypoints that we don't
        // https://github.com/pypa/virtualenv/blob/025e96fbad37f85617364002ae2a0064b09fc984/src/virtualenv/seed/embed/via_app_data/pip_install/base.py#L74-L95
        let ini_text = fs::read_to_string(
            site_packages
                .join(format!("{name}-{version}.dist-info"))
                .join("entry_points.txt"),
        )
        .with_context(|| format!("{name} should have an entry_points.txt"))?;
        let entry_points_mapping = Ini::new_cs()
            .read(ini_text)
            .map_err(|err| format_err!("{name} entry_points.txt is invalid: {}", err))?;
        for (key, value) in entry_points_mapping
            .get("console_scripts")
            .cloned()
            .unwrap_or_default()
        {
            let (import_from, function) = value
                .as_ref()
                .and_then(|value| value.split_once(':'))
                .ok_or_else(|| {
                    format_err!("{name} entry_points.txt {key} has an invalid value {value:?}")
                })?;
            let launcher = bin_dir.join(key);
            let launcher_script = bare::unix_launcher_script(venv_python, import_from, function);
            fs::write(&launcher, launcher_script)?;
            // We need to make the launcher executable
            #[cfg(target_family = "unix")]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(launcher, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

/// https://stackoverflow.com/a/65192210/3549270
#[cfg(not(feature = "install"))]
pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src.as_ref())? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Template for the console scripts in the `bin` directory
#[cfg(not(feature = "install"))]
pub fn unix_launcher_script(python: &Utf8Path, import_from: &str, function: &str) -> String {
    format!(
        r#"#!{python}
    # -*- coding: utf-8 -*-
import re
import sys
from {import_from} import {function}
if __name__ == '__main__':
    sys.argv[0] = re.sub(r'(-script\.pyw|\.exe)?$', '', sys.argv[0])
    sys.exit({function}())
"#,
        python = python,
        import_from = import_from,
        function = function
    )
}
