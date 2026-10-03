// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Contrôle de version distante de la base — `checkDBVer.php`.
//!
//! Portage de la partie réseau de
//! `module/version/controle_version_database_api.py`. La lecture et l'écriture
//! de `bdd/last_update.txt` vivent dans [`ygo_core::version`] (règle R1) ;
//! seul l'appel d'API est ici.

use ygo_core::version::InfoVersion;

use crate::error::Result;
use crate::http::ClientHttp;

/// Point d'accès de contrôle de version YGOPRODeck.
pub const CHECK_DB_VER_URL: &str = "https://db.ygoprodeck.com/api/v7/checkDBVer.php";

/// Interroge la version distante de la base.
///
/// L'API renvoie une liste ; le Python n'en consulte que la première entrée.
pub async fn version_distante(client: &ClientHttp) -> Result<Vec<InfoVersion>> {
    let infos: Vec<InfoVersion> = client.get_json(CHECK_DB_VER_URL).await?;
    if let Some(premiere) = infos.first() {
        tracing::info!(
            version = %premiere.database_version,
            maj = %premiere.last_update,
            "version distante de la base"
        );
    }
    Ok(infos)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn la_reponse_de_l_api_se_deserialise() {
        // Forme réelle observée dans `bdd/last_update.txt` de la V1.0.3.
        let json = r#"[{"database_version":"146.68","last_update":"2026-08-21 00:00:28"}]"#;
        let infos: Vec<InfoVersion> = serde_json::from_str(json).unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].database_version, "146.68");
    }

    #[test]
    fn l_url_est_celle_du_python() {
        assert_eq!(
            CHECK_DB_VER_URL,
            "https://db.ygoprodeck.com/api/v7/checkDBVer.php"
        );
    }
}
