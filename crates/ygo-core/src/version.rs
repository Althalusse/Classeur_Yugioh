// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Version connue de la base de référence — `bdd/last_update.txt`.
//!
//! Portage de la partie « fichier » de
//! `module/version/controle_version_database_api.py`. L'appel réseau à
//! `checkDBVer.php` vit dans `ygo-sources` (règle R1).
//!
//! # Piège de portage
//!
//! Malgré son extension `.txt`, **ce fichier contient du JSON** — une liste
//! d'objets, telle que la renvoie l'API YGOPRODeck :
//!
//! ```json
//! [
//!   {
//!     "database_version": "146.68",
//!     "last_update": "2026-08-21 00:00:28"
//!   }
//! ]
//! ```
//!
//! Le lire comme une chaîne fait échouer la comparaison de version en silence,
//! et refait l'initialisation à chaque démarrage. Le cahier des charges décrit ce
//! fichier comme « version distante connue de la base » sans préciser son
//! format — d'où cette note.
//!
//! Le Python accepte aussi un objet seul (non enveloppé dans une liste) et le
//! normalise en liste ; on fait de même.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Une entrée de version telle que renvoyée par `checkDBVer.php`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoVersion {
    /// Version de la base distante, p. ex. `"146.68"`.
    pub database_version: String,
    /// Horodatage de la dernière mise à jour distante, p. ex.
    /// `"2026-08-21 00:00:28"`.
    pub last_update: String,
}

/// Lit `bdd/last_update.txt`.
///
/// Retourne `None` si le fichier est absent, vide ou illisible — comme
/// `load_last_update_file()`. Un fichier au JSON invalide est **supprimé**,
/// exactement comme en Python : c'est ce qui permet à un fichier corrompu de
/// se réparer tout seul au démarrage suivant.
pub fn charger(chemin: impl AsRef<Path>) -> Option<Vec<InfoVersion>> {
    let chemin = chemin.as_ref();
    let texte = std::fs::read_to_string(chemin).ok()?;

    match serde_json::from_str::<serde_json::Value>(&texte) {
        Ok(valeur) => {
            let liste = match valeur {
                serde_json::Value::Array(items) => items,
                autre => vec![autre],
            };
            let infos: Vec<InfoVersion> = liste
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect();
            (!infos.is_empty()).then_some(infos)
        }
        Err(e) => {
            tracing::warn!(
                chemin = %chemin.display(),
                erreur = %e,
                "last_update.txt corrompu — suppression pour permettre sa reconstruction"
            );
            let _ = std::fs::remove_file(chemin);
            None
        }
    }
}

/// Écrit `bdd/last_update.txt` (JSON, indentation de 2 espaces).
pub fn enregistrer(chemin: impl AsRef<Path>, infos: &[InfoVersion]) -> Result<()> {
    let chemin: PathBuf = chemin.as_ref().to_path_buf();
    if let Some(parent) = chemin.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        }
    }
    let texte = serde_json::to_string_pretty(infos).map_err(|e| CoreError::json(&chemin, e))?;
    std::fs::write(&chemin, texte).map_err(|e| CoreError::io(&chemin, e))
}

/// Version locale connue, si elle existe.
///
/// Reprend la logique de `check_for_updates()` : seule la **première** entrée
/// de la liste est consultée.
pub fn version_locale(infos: Option<&[InfoVersion]>) -> Option<&str> {
    infos?.first().map(|i| i.database_version.as_str())
}

/// Une mise à jour est-elle nécessaire ?
///
/// Reprend la comparaison du Python : **une simple inégalité de chaînes**, pas
/// une comparaison sémantique de version. `"146.68"` et `"146.7"` sont donc
/// considérées différentes sans être ordonnées.
pub fn mise_a_jour_necessaire(locale: Option<&str>, distante: &str) -> bool {
    locale != Some(distante)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Contenu réel de `V1.0.3/bdd/last_update.txt`.
    const REEL: &str = r#"[
  {
    "database_version": "146.68",
    "last_update": "2026-08-21 00:00:28"
  }
]"#;

    #[test]
    fn lit_le_fichier_reel() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("last_update.txt");
        std::fs::write(&chemin, REEL).unwrap();

        let infos = charger(&chemin).unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(version_locale(Some(&infos)), Some("146.68"));
        assert_eq!(
            infos.first().map(|i| i.last_update.as_str()),
            Some("2026-08-21 00:00:28")
        );
    }

    #[test]
    fn accepte_un_objet_seul_et_le_normalise_en_liste() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("last_update.txt");
        std::fs::write(
            &chemin,
            br#"{"database_version": "1.0", "last_update": "2026-01-01 00:00:00"}"#,
        )
        .unwrap();

        let infos = charger(&chemin).unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(version_locale(Some(&infos)), Some("1.0"));
    }

    #[test]
    fn fichier_absent_donne_none() {
        assert!(charger("/chemin/qui/n/existe/pas.txt").is_none());
        assert!(version_locale(None).is_none());
    }

    /// JSON syntaxiquement invalide : le Python lève `JSONDecodeError` et
    /// supprime le fichier pour qu'il se reconstruise au démarrage suivant.
    #[test]
    fn json_invalide_est_supprime() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("last_update.txt");
        std::fs::write(&chemin, b"{ ceci n'est pas du JSON").unwrap();

        assert!(charger(&chemin).is_none());
        assert!(!chemin.exists(), "le fichier corrompu doit être supprimé");
    }

    /// JSON **valide** mais de forme inattendue : `146.68` est un nombre JSON
    /// parfaitement légal. Le Python ne lève donc pas `JSONDecodeError`, ne
    /// supprime pas le fichier, et échoue plus loin en tentant
    /// `local_info[0]["database_version"]` — échec avalé par le `try` de
    /// `check_for_updates()`, qui conclut « pas de mise à jour ».
    ///
    /// Côté Rust, l'entrée non conforme est simplement écartée : le résultat
    /// observable est le même (aucune version locale connue), sans le détour
    /// par une exception.
    #[test]
    fn json_valide_mais_de_forme_inattendue_est_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("last_update.txt");
        std::fs::write(&chemin, b"146.68").unwrap();

        assert!(charger(&chemin).is_none());
        assert!(
            chemin.exists(),
            "le fichier n'est pas syntaxiquement corrompu"
        );
    }

    #[test]
    fn comparaison_de_version_est_une_egalite_de_chaines() {
        assert!(mise_a_jour_necessaire(None, "146.68"));
        assert!(mise_a_jour_necessaire(Some("146.67"), "146.68"));
        assert!(!mise_a_jour_necessaire(Some("146.68"), "146.68"));
        // Pas de comparaison sémantique : 146.7 != 146.70, et c'est voulu.
        assert!(mise_a_jour_necessaire(Some("146.7"), "146.70"));
    }

    #[test]
    fn aller_retour_disque() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("bdd").join("last_update.txt");

        let infos = vec![InfoVersion {
            database_version: "146.68".into(),
            last_update: "2026-08-21 00:00:28".into(),
        }];
        enregistrer(&chemin, &infos).unwrap();
        assert_eq!(charger(&chemin).unwrap(), infos);
    }
}
