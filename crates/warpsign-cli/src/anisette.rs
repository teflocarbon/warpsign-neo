//! `warpsign anisette`: make anisette headers on this computer, with
//! Apple's Android ADI libraries in Unicorn. A probe for now: it shows
//! which headers came back and their lengths, never the values.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use warpsign_core::anisette::apk::{self, Libraries};
use warpsign_core::anisette::unicorn::Unicorn;
use warpsign_core::anisette::{Anisette, AnisetteState};

use crate::store::{Store, write_private};

fn cached_libraries(dir: &Path) -> Option<Libraries> {
    Some(Libraries {
        store_services_core: std::fs::read(dir.join("libstoreservicescore.so")).ok()?,
        core_adi: std::fs::read(dir.join("libCoreADI.so")).ok()?,
    })
}

pub fn probe(store: &Store, unicorn: &Path, libraries: Option<PathBuf>) -> Result<()> {
    let transport = crate::transport()?;
    let home = store.dir().join("anisette");
    let library_dir = libraries.unwrap_or_else(|| home.join("lib"));

    let started = Instant::now();
    let unicorn = Arc::new(Unicorn::load(unicorn).map_err(anyhow::Error::msg)?);
    println!("Unicorn loaded ({} ms)", started.elapsed().as_millis());

    let libraries = match cached_libraries(&library_dir) {
        Some(libraries) => {
            println!("Libraries from {}", library_dir.display());
            libraries
        }
        None => {
            println!("Fetching Apple's libraries from the Apple Music APK...");
            let started = Instant::now();
            let libraries = apk::download(transport.as_ref())?;
            std::fs::create_dir_all(&library_dir)
                .with_context(|| format!("creating {}", library_dir.display()))?;
            std::fs::write(
                library_dir.join("libstoreservicescore.so"),
                &libraries.store_services_core,
            )?;
            std::fs::write(library_dir.join("libCoreADI.so"), &libraries.core_adi)?;
            println!(
                "Saved to {} ({} s)",
                library_dir.display(),
                started.elapsed().as_secs()
            );
            libraries
        }
    };
    println!(
        "  libstoreservicescore.so {} bytes, libCoreADI.so {} bytes",
        libraries.store_services_core.len(),
        libraries.core_adi.len()
    );

    let state_path = home.join("state.json");
    let state = match std::fs::read(&state_path) {
        Ok(data) => serde_json::from_slice(&data)
            .with_context(|| format!("reading {}", state_path.display()))?,
        Err(_) => AnisetteState::generate(),
    };

    let started = Instant::now();
    let mut anisette = Anisette::new(unicorn, &libraries, state)?;
    println!("ADI loaded ({} ms)", started.elapsed().as_millis());
    let missing = anisette.missing_imports();
    if !missing.is_empty() {
        println!(
            "  {} imports not provided by the emulated libc (fine unless called)",
            missing.len()
        );
    }

    let save = |anisette: &Anisette| -> Result<()> {
        std::fs::create_dir_all(&home)?;
        write_private(&state_path, &serde_json::to_vec(anisette.state())?)
            .with_context(|| format!("writing {}", state_path.display()))
    };

    if anisette.is_provisioned()? {
        println!("Already provisioned");
    } else {
        println!("Provisioning a new machine with Apple (no Apple ID involved)...");
        let started = Instant::now();
        anisette.provision(transport.as_ref())?;
        save(&anisette)?;
        println!("Provisioned ({} ms)", started.elapsed().as_millis());
    }

    let started = Instant::now();
    let headers = anisette.headers()?;
    save(&anisette)?;
    println!(
        "Headers ({} ms), values not shown:",
        started.elapsed().as_millis()
    );
    for (name, value) in &headers {
        println!("  {name}: {} chars", value.len());
    }
    println!(
        "The identity is saved in {} (secret: keep it private).",
        state_path.display()
    );
    Ok(())
}
