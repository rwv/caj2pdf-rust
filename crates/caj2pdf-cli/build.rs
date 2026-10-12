// SPDX-License-Identifier: MIT

// Keep registry sources small. Only native CLI binaries embed the pinned
// pack; the core and raw WASM contain metadata, never the font programs.
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::Read,
    path::PathBuf,
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=font-pack.json");
    println!("cargo:rerun-if-env-changed=CAJ2PDF_FONT_PACK");
    if !env::var("CARGO_CFG_TARGET_FAMILY")
        .unwrap_or_default()
        .split(',')
        .any(|f| matches!(f, "unix" | "windows"))
    {
        return;
    }
    let meta: serde_json::Value = serde_json::from_str(include_str!("font-pack.json")).unwrap();
    let expected_size = meta["size"].as_u64().unwrap();
    let expected_hash = meta["sha256"].as_str().unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("fonts.bin");
    let verify = |path: &PathBuf| -> bool {
        let Ok(mut file) = File::open(path) else {
            return false;
        };
        if file.metadata().unwrap().len() != expected_size {
            return false;
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let count = file.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            == expected_hash
    };
    if let Some(local) = env::var_os("CAJ2PDF_FONT_PACK") {
        let local = PathBuf::from(local);
        println!("cargo:rerun-if-changed={}", local.display());
        fs::copy(local, &output).unwrap();
        assert!(
            verify(&output),
            "copied CAJ2PDF_FONT_PACK must match the pinned checksum"
        );
    } else if !verify(&output) {
        let temporary = output.with_extension("download");
        let status = Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--retry",
                "3",
                "--connect-timeout",
                "30",
                "--max-time",
                "600",
                "--max-filesize",
                &expected_size.to_string(),
                "--output",
            ])
            .arg(&temporary)
            .arg(meta["url"].as_str().unwrap())
            .status()
            .expect("source builds need curl or an offline CAJ2PDF_FONT_PACK");
        assert!(
            status.success() && verify(&temporary),
            "font pack download failed or checksum mismatch; supply a verified CAJ2PDF_FONT_PACK for an offline build"
        );
        fs::rename(temporary, &output).unwrap();
    }
}
