// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Journalisation — `logs/app.log`.
//!
//! Portage de `module/logger_app.py` : `tracing` + `tracing-appender` en
//! remplacement de `logging` + `RotatingFileHandler`.
//!
//! Le format des lignes reprend celui du Python, pour que les journaux des deux
//! versions restent comparables pendant la migration :
//!
//! ```text
//! 2026-08-25 21:28:32  [INFO ]  Démarrage de l'application
//! ```
//!
//! C'est plus qu'un souci d'esthétique : les compteurs journalisés par la
//! V1.0.4 (« RA05 : 4 tirage(s) ajouté(s), 134 illustration(s) … ») servent
//! d'assertions de non-régression au portage. Un format identique permet de
//! les extraire avec le même outil des deux côtés.

use std::path::Path;

use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::error::{CoreError, Result};

/// Format d'horodatage du Python : `2026-08-25 21:28:32`.
const FORMAT_HORODATAGE: &str = "%Y-%m-%d %H:%M:%S";

/// Garde de journalisation.
///
/// **À conserver vivante pour toute la durée du programme** : sa destruction
/// vide et ferme le fichier de journal. La lâcher trop tôt fait perdre les
/// dernières lignes — typiquement celles qui expliquent un arrêt.
#[derive(Debug)]
pub struct GardeJournal {
    _garde: tracing_appender::non_blocking::WorkerGuard,
}

/// Installe la journalisation applicative.
///
/// - Écrit dans `<dossier>/app.log`, avec rotation **quotidienne**.
/// - Duplique sur la sortie d'erreur standard, utile pour `ygo-cli`.
/// - Le niveau se règle par la variable d'environnement `YGO_LOG`
///   (syntaxe `EnvFilter`), et vaut `info` par défaut.
///
/// Ne doit être appelée qu'une fois par processus ; un second appel renvoie une
/// erreur plutôt que de paniquer.
pub fn installer(dossier_logs: impl AsRef<Path>) -> Result<GardeJournal> {
    let dossier = dossier_logs.as_ref();
    std::fs::create_dir_all(dossier).map_err(|e| CoreError::io(dossier, e))?;

    let fichier = tracing_appender::rolling::daily(dossier, "app.log");
    let (non_bloquant, garde) = tracing_appender::non_blocking(fichier);

    let filtre = EnvFilter::try_from_env("YGO_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let horodatage = ChronoLocal::new(FORMAT_HORODATAGE.to_owned());

    let couche_fichier = tracing_subscriber::fmt::layer()
        .with_writer(non_bloquant)
        .with_ansi(false)
        .with_target(false)
        .with_timer(horodatage.clone());

    let couche_console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_timer(horodatage);

    tracing_subscriber::registry()
        .with(filtre)
        .with(couche_fichier)
        .with(couche_console)
        .try_init()
        .map_err(|e| {
            CoreError::RacineIntrouvable(format!("journalisation déjà installée : {e}"))
        })?;

    Ok(GardeJournal { _garde: garde })
}

/// Écrit l'en-tête de session, à l'identique du Python.
///
/// ```text
/// ======================================================================
/// Session démarrée — 2026-08-25T21:28:46
/// Version : 2.0.0-alpha.1
/// Platform : windows
/// ======================================================================
/// ```
pub fn entete_session(version: &str) {
    const BARRE: &str = "======================================================================";
    tracing::info!("{BARRE}");
    tracing::info!("Session démarrée — {}", horodatage_iso());
    tracing::info!("Version : {version}");
    tracing::info!("Platform : {}", std::env::consts::OS);
    tracing::info!("{BARRE}");
}

fn horodatage_iso() -> String {
    // Même forme que le Python : `datetime.now().isoformat(timespec='seconds')`.
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// `installer` pose le collecteur **global** du processus : deux tests
    /// distincts se marcheraient dessus selon l'ordre d'exécution. Ils sont
    /// donc réunis ici, où la séquence est déterministe.
    ///
    /// (C'est exactement le genre de test que `cargo nextest` — un processus
    /// par test — rendrait séparable. À reconsidérer quand il sera en place.)
    #[test]
    fn installation_du_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let dossier = tmp.path().join("logs");

        // 1. Premier appel : crée le dossier, écrit dans app.log.
        let garde = installer(&dossier).unwrap();
        tracing::info!("ligne de test");
        entete_session("2.0.0-test");
        drop(garde); // force le vidage du tampon

        assert!(dossier.is_dir());
        let fichiers: Vec<_> = std::fs::read_dir(&dossier)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            fichiers.iter().any(|f| f.starts_with("app.log")),
            "un fichier app.log* doit exister, trouvé : {fichiers:?}"
        );

        // 2. Second appel : renvoie une erreur, ne panique jamais.
        let second = installer(tmp.path().join("autre"));
        assert!(
            second.is_err(),
            "la journalisation est globale au processus : le second appel doit échouer proprement"
        );
    }
}
