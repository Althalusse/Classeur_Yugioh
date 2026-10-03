// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Ce qu'il y a à télécharger, décidé **sans réseau ni base**.
//!
//! Portage de `file_attente_classeur.lister_images_a_telecharger`, moins la
//! lecture SQLite : l'appelant fournit les lignes, cette fonction rend le plan.
//! C'est ce qui la rend éprouvable en quelques lignes, là où le Python
//! demandait une base, un dossier d'images et un classeur peuplé.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ygo_core::config::SourceImage;
use ygo_core::image_source;

/// Une ligne de classeur, réduite à ce dont le plan a besoin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneImage {
    /// `card_image_url` — l'URL stockée.
    pub card_image_url: Option<String>,
    /// `card_image_id` — négatif pour un artwork Yugipedia.
    pub card_image_id: Option<i64>,
}

/// Une image à télécharger : deux sources et une destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cible {
    /// L'URL de la source active.
    pub url_primaire: String,
    /// L'URL de l'autre source, essayée si la première échoue.
    pub url_repli: Option<String>,
    /// Où le fichier doit atterrir.
    pub destination: PathBuf,
}

/// Ce qu'une passe a fait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Images téléchargées, par la source primaire ou celle de repli.
    pub reussies: usize,
    /// Images qu'aucune source n'a rendues — un substitut est posé.
    pub echecs: usize,
    /// Images déjà présentes, écartées par le plan.
    pub deja_presentes: usize,
}

/// Le nom de fichier d'une URL — la dernière composante de son chemin.
///
/// Reprend `os.path.basename(urlparse(url).path)`. Les paramètres de requête
/// et l'ancre sont écartés : `…/12345.jpg?v=2#x` donne `12345.jpg`.
#[must_use]
pub fn nom_de_fichier(url: &str) -> Option<String> {
    let sans_ancre = url.split('#').next().unwrap_or(url);
    let sans_requete = sans_ancre.split('?').next().unwrap_or(sans_ancre);
    let dernier = sans_requete.rsplit('/').next()?;
    if dernier.is_empty() {
        None
    } else {
        Some(dernier.to_owned())
    }
}

/// Décide quelles images télécharger.
///
/// - `lignes` : les lignes du classeur, lues par l'appelant ;
/// - `source` : la source d'images configurée ;
/// - `dossier_small` : `img/small`, pour les cartes qui ont un identifiant ;
/// - `dossier_classeur` : `img/<CODE>`, pour celles qui n'en ont pas ;
/// - `chercher_repli` : la recherche d'URL de repli dans `cardinfo.db`,
///   injectée parce qu'elle relève de SQLite ;
/// - `a_retelecharger` : dit d'un fichier **présent** s'il faut le reprendre —
///   c'est là que la détection de substitut se branche.
///
/// # La déduplication n'est pas une optimisation
///
/// Plusieurs lignes d'un set partagent le même `card_image_id` — la même carte
/// en sept raretés montre la même illustration — donc la même destination. En
/// séquentiel, le doublon ne faisait que réécrire le fichier. **En parallèle,
/// deux tâches écrivant le même `.part` se corrompent mutuellement.** La
/// déduplication est donc une condition de correction, et le Python la
/// documente comme telle.
///
/// L'ordre du classeur est conservé : le premier arrivé gagne, ce qui rend le
/// plan reproductible.
pub fn planifier<F, G>(
    lignes: &[LigneImage],
    source: SourceImage,
    dossier_small: &Path,
    dossier_classeur: &Path,
    chercher_repli: F,
    a_retelecharger: G,
) -> (Vec<Cible>, usize)
where
    F: Fn(i64) -> Option<String>,
    G: Fn(&Path) -> Retour,
{
    let mut cibles = Vec::new();
    let mut vues: HashSet<PathBuf> = HashSet::new();
    let mut deja_presentes = 0;

    for ligne in lignes {
        let Some(url_primaire) =
            image_source::url_image(source, ligne.card_image_url.as_deref(), ligne.card_image_id)
        else {
            continue;
        };
        let Some(nom) = nom_de_fichier(&url_primaire) else {
            continue;
        };

        // Une carte sans identifiant n'a pas sa place dans `img/small`, qui est
        // indexé par identifiant : elle va dans le dossier du classeur. C'est
        // le `if stored_id:` du Python, où `0` compte comme absent.
        let destination = match ligne.card_image_id {
            Some(id) if id != 0 => dossier_small.join(&nom),
            _ => dossier_classeur.join(&nom),
        };

        if !vues.insert(destination.clone()) {
            continue;
        }

        match a_retelecharger(&destination) {
            Retour::Presente => {
                deja_presentes += 1;
                continue;
            }
            Retour::AReprendre | Retour::Absente => {}
        }

        let url_repli = ligne
            .card_image_id
            .and_then(|id| image_source::url_repli(source, Some(id), &chercher_repli));

        cibles.push(Cible {
            url_primaire,
            url_repli,
            destination,
        });
    }

    (cibles, deja_presentes)
}

/// L'état d'un fichier de destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retour {
    /// Le fichier n'existe pas.
    Absente,
    /// Le fichier existe et convient.
    Presente,
    /// Le fichier existe mais c'est un substitut : on retente.
    AReprendre,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn ligne(url: &str, id: Option<i64>) -> LigneImage {
        LigneImage {
            card_image_url: if url.is_empty() {
                None
            } else {
                Some(url.to_owned())
            },
            card_image_id: id,
        }
    }

    fn small() -> PathBuf {
        PathBuf::from("/img/small")
    }
    fn par_classeur() -> PathBuf {
        PathBuf::from("/img/RA02")
    }

    #[test]
    fn le_nom_de_fichier_ignore_requete_et_ancre() {
        assert_eq!(
            nom_de_fichier("https://x.tld/images/cards/12345.jpg").as_deref(),
            Some("12345.jpg")
        );
        assert_eq!(
            nom_de_fichier("https://x.tld/a/b/Carte-RA02-EN-UR-1E.png?v=2#x").as_deref(),
            Some("Carte-RA02-EN-UR-1E.png")
        );
        assert_eq!(nom_de_fichier("https://x.tld/"), None);
        assert_eq!(nom_de_fichier(""), None);
    }

    #[test]
    fn une_carte_avec_identifiant_va_dans_img_small() {
        let lignes = vec![ligne("https://x.tld/12345.jpg", Some(12345))];
        let (cibles, _) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::Absente,
        );
        assert_eq!(cibles.len(), 1);
        assert_eq!(cibles[0].destination, small().join("12345.jpg"));
    }

    /// `0` compte comme absent — c'est le `if stored_id:` du Python.
    #[test]
    fn une_carte_sans_identifiant_va_dans_le_dossier_du_classeur() {
        for id in [None, Some(0)] {
            let lignes = vec![ligne("https://x.tld/Carte.png", id)];
            let (cibles, _) = planifier(
                &lignes,
                SourceImage::Yugipedia,
                &small(),
                &par_classeur(),
                |_| None,
                |_| Retour::Absente,
            );
            assert_eq!(cibles.len(), 1, "id = {id:?}");
            assert_eq!(cibles[0].destination, par_classeur().join("Carte.png"));
        }
    }

    /// La déduplication est une condition de correction, pas une optimisation :
    /// deux tâches parallèles écrivant le même `.part` se corrompraient.
    #[test]
    fn deux_raretes_de_la_meme_carte_ne_donnent_qu_une_cible() {
        let lignes = vec![
            ligne("https://x.tld/12345.jpg", Some(12345)),
            ligne("https://x.tld/12345.jpg", Some(12345)),
            ligne("https://x.tld/12345.jpg", Some(12345)),
        ];
        let (cibles, deja) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::Absente,
        );
        assert_eq!(cibles.len(), 1);
        assert_eq!(deja, 0, "les doublons ne comptent pas comme présents");
    }

    #[test]
    fn une_image_deja_presente_est_ecartee_et_comptee() {
        let lignes = vec![ligne("https://x.tld/12345.jpg", Some(12345))];
        let (cibles, deja) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::Presente,
        );
        assert!(cibles.is_empty());
        assert_eq!(deja, 1);
    }

    /// Un substitut posé par un échec précédent doit être retenté — sans quoi
    /// une carte indisponible un jour le resterait pour toujours.
    #[test]
    fn un_substitut_est_retente() {
        let lignes = vec![ligne("https://x.tld/12345.jpg", Some(12345))];
        let (cibles, deja) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::AReprendre,
        );
        assert_eq!(cibles.len(), 1);
        assert_eq!(deja, 0);
    }

    /// L'artwork Yugipedia — identifiant négatif — sert son URL stockée quelle
    /// que soit la source active, et n'a **pas** de repli : il n'existe qu'une
    /// seule adresse pour lui.
    #[test]
    fn un_artwork_yugipedia_n_a_pas_de_repli() {
        let lignes = vec![ligne(
            "https://ms.yugipedia.com//3/35/GhostOgre-RA02-EN-CR-1E.png",
            Some(-1074569406),
        )];
        for source in [SourceImage::Ygoprodeck, SourceImage::Yugipedia] {
            let (cibles, _) = planifier(
                &lignes,
                source,
                &small(),
                &par_classeur(),
                |_| Some("https://autre".to_owned()),
                |_| Retour::Absente,
            );
            assert_eq!(cibles.len(), 1);
            assert_eq!(
                cibles[0].url_primaire,
                "https://ms.yugipedia.com//3/35/GhostOgre-RA02-EN-CR-1E.png"
            );
            assert_eq!(cibles[0].url_repli, None, "source {source:?}");
        }
    }

    #[test]
    fn le_repli_est_demande_pour_une_carte_ordinaire() {
        let lignes = vec![ligne("https://x.tld/12345.jpg", Some(12345))];
        let (cibles, _) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |id| {
                assert_eq!(id, 12345);
                Some("https://ms.yugipedia.com//a/b/Carte.png".to_owned())
            },
            |_| Retour::Absente,
        );
        assert_eq!(
            cibles[0].url_repli.as_deref(),
            Some("https://ms.yugipedia.com//a/b/Carte.png")
        );
    }

    #[test]
    fn une_ligne_sans_url_exploitable_est_ignoree() {
        let lignes = vec![ligne("", None), ligne("", Some(0))];
        let (cibles, deja) = planifier(
            &lignes,
            SourceImage::Yugipedia,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::Absente,
        );
        assert!(cibles.is_empty());
        assert_eq!(deja, 0);
    }

    #[test]
    fn l_ordre_du_classeur_est_conserve() {
        let lignes = vec![
            ligne("https://x.tld/3.jpg", Some(3)),
            ligne("https://x.tld/1.jpg", Some(1)),
            ligne("https://x.tld/2.jpg", Some(2)),
        ];
        let (cibles, _) = planifier(
            &lignes,
            SourceImage::Ygoprodeck,
            &small(),
            &par_classeur(),
            |_| None,
            |_| Retour::Absente,
        );
        let noms: Vec<String> = cibles
            .iter()
            .map(|c| c.destination.file_name().unwrap().to_string_lossy().into())
            .collect();
        assert_eq!(noms, ["3.jpg", "1.jpg", "2.jpg"]);
    }
}
