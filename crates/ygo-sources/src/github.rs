// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les Releases GitHub du dépôt — d'où vient la mise à jour de l'application.
//!
//! Une seule requête, `GET /repos/Althalusse/Classeur_Yugioh/releases`, sans
//! compte : GitHub en autorise soixante par heure et par adresse, et
//! l'application n'en fait au plus qu'une par jour (cf.
//! `ygo_app::maj_appli`). Rien n'est mis en cache disque : une Release
//! publiée doit se voir au prochain contrôle.
//!
//! Les brouillons (`draft`) ne sont jamais rendus par l'API à un visiteur
//! anonyme ; ils sont écartés ici quand même, par principe.

use serde::Deserialize;

use crate::error::Result;
use crate::http::ClientHttp;

/// Le dépôt public de l'application.
pub const DEPOT: &str = "Althalusse/Classeur_Yugioh";

/// Combien de Releases lire : la plus récente stable peut suivre plusieurs
/// versions de test.
const PAR_PAGE: u32 = 20;

/// Une Release, réduite à ce que la mise à jour lit.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Release {
    /// Le tag — `v2.0.0-alpha.2`.
    pub tag_name: String,
    /// Le titre affiché.
    #[serde(default)]
    pub name: Option<String>,
    /// La description — les nouveautés, en Markdown.
    #[serde(default)]
    pub body: Option<String>,
    /// Marquée « pre-release » ?
    #[serde(default)]
    pub prerelease: bool,
    /// Brouillon ?
    #[serde(default)]
    pub draft: bool,
    /// La page de la Release sur GitHub.
    #[serde(default)]
    pub html_url: String,
    /// Les fichiers joints.
    #[serde(default)]
    pub assets: Vec<Fichier>,
}

/// Un fichier joint à une Release.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Fichier {
    /// Nom du fichier — `ygo-ui.exe`.
    pub name: String,
    /// Adresse de téléchargement direct.
    pub browser_download_url: String,
    /// Taille en octets.
    #[serde(default)]
    pub size: u64,
    /// L'empreinte que GitHub calcule lui-même à la publication —
    /// `sha256:4f69…`. Absente des fichiers joints avant mi-2025.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Release {
    /// Le fichier joint de ce nom, s'il y en a un.
    #[must_use]
    pub fn fichier(&self, nom: &str) -> Option<&Fichier> {
        self.assets
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(nom))
    }
}

/// L'adresse de l'API qui liste les Releases du dépôt.
#[must_use]
pub fn url_releases() -> String {
    format!("https://api.github.com/repos/{DEPOT}/releases?per_page={PAR_PAGE}")
}

/// Les Releases publiées du dépôt, de la plus récente à la plus ancienne.
///
/// # Errors
///
/// Rend une erreur sur une panne réseau, un statut d'erreur (dont le 403 d'un
/// quota GitHub dépassé) ou une réponse illisible.
pub async fn releases(client: &ClientHttp) -> Result<Vec<Release>> {
    let toutes: Vec<Release> = client.get_json(&url_releases()).await?;
    Ok(toutes.into_iter().filter(|r| !r.draft).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    /// La forme réelle de l'API, champs inutiles compris : ils sont ignorés.
    #[test]
    fn une_release_se_lit_depuis_la_reponse_de_l_api() {
        let json = r###"[{
            "url": "https://api.github.com/repos/Althalusse/Classeur_Yugioh/releases/1",
            "html_url": "https://github.com/Althalusse/Classeur_Yugioh/releases/tag/v2.0.0-alpha.2",
            "tag_name": "v2.0.0-alpha.2",
            "name": "v2.0.0-alpha.2 — mise à jour intégrée",
            "draft": false,
            "prerelease": true,
            "body": "## Quoi de neuf\n- mise à jour",
            "assets": [
                {"name": "ygo-ui.exe", "size": 15800000,
                 "digest": "sha256:4f69f78aaefbabe774cfdb142c13d0000000000000000000000000000000000",
                 "browser_download_url": "https://github.com/Althalusse/Classeur_Yugioh/releases/download/v2.0.0-alpha.2/ygo-ui.exe"},
                {"name": "ygo-ui.exe.sha256", "size": 77,
                 "browser_download_url": "https://github.com/Althalusse/Classeur_Yugioh/releases/download/v2.0.0-alpha.2/ygo-ui.exe.sha256"}
            ]
        }]"###;
        let r: Vec<Release> = serde_json::from_str(json).unwrap();
        assert_eq!(r[0].tag_name, "v2.0.0-alpha.2");
        assert!(r[0].prerelease);
        assert_eq!(r[0].fichier("YGO-UI.EXE").unwrap().size, 15_800_000);
        assert!(r[0]
            .fichier("ygo-ui.exe")
            .unwrap()
            .digest
            .as_deref()
            .unwrap()
            .starts_with("sha256:4f69"));
        assert!(r[0].fichier("ygo-ui.exe.sha256").unwrap().digest.is_none());
        assert!(r[0].fichier("ygo-cli.exe").is_none());
    }

    #[test]
    fn l_adresse_vise_le_depot_public() {
        assert_eq!(
            url_releases(),
            "https://api.github.com/repos/Althalusse/Classeur_Yugioh/releases?per_page=20"
        );
    }
}
