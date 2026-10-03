// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le téléchargement lui-même : écriture atomique, repli, parallélisme borné.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use ygo_sources::ClientHttp;

use crate::cache::{est_placeholder, SignaturePlaceholder};
use crate::plan::{Bilan, Cible};

/// Nombre de téléchargements simultanés.
///
/// `_DL_PARALLELISM` du Python. Le débit reste borné par le quota **global**
/// du client HTTP — quinze requêtes par seconde vers les images YGOPRODeck,
/// une toutes les 1,1 s vers Yugipedia. Ce parallélisme ne peut donc pas
/// dépasser la limite : il masque la latence réseau, il ne l'outrepasse pas.
pub const PARALLELISME: usize = 6;

/// En dessous de cette taille, la réponse n'est pas une carte.
///
/// Le contrôle existe parce que Yugipedia rend parfois une page d'erreur
/// **en 200 OK**. Sans lui, cette page écraserait un fichier valide, ou
/// s'installerait comme si elle était l'image.
pub const TAILLE_MINIMALE: u64 = 200;

/// Ce qui a pu mal tourner.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// Écriture impossible.
    #[error("écriture de {chemin} : {source}")]
    Ecriture {
        /// Le fichier visé.
        chemin: PathBuf,
        /// La cause.
        source: std::io::Error,
    },
}

/// Écrit un fichier de façon **atomique** : `.part`, `fsync`, puis `rename`.
///
/// # Ce que cette fonction empêche
///
/// Le téléchargeur écrit pendant que l'interface relit. Si le fichier était
/// écrit directement à son emplacement final, un lecteur pourrait l'ouvrir à
/// moitié écrit et n'obtenir qu'une image tronquée. Dans le Python, ce cas
/// faisait planter libjpeg **au niveau C** : l'application disparaissait sans
/// exception, sans trace dans les journaux, et de façon aléatoire puisqu'elle
/// dépendait du calage exact entre lecture et écriture.
///
/// `rename` sur un même volume est atomique : un lecteur concurrent voit
/// toujours un fichier complet — l'ancien ou le nouveau, jamais un mélange.
/// Le `fsync` avant la bascule garantit que le contenu est bien sur le disque
/// quand `rename` rend la main, et non seulement dans le cache d'écriture.
///
/// En cas d'échec, le `.part` est effacé et le fichier cible reste **intact**.
pub fn ecrire_atomique(chemin: &Path, octets: &[u8]) -> Result<(), ImageError> {
    let erreur = |source| ImageError::Ecriture {
        chemin: chemin.to_path_buf(),
        source,
    };
    if let Some(parent) = chemin.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(erreur)?;
        }
    }
    let temporaire = chemin.with_extension(format!(
        "{}part",
        chemin
            .extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));

    let ecrire = || -> std::io::Result<()> {
        let mut fichier = std::fs::File::create(&temporaire)?;
        fichier.write_all(octets)?;
        fichier.flush()?;
        fichier.sync_all()?;
        drop(fichier);
        std::fs::rename(&temporaire, chemin)
    };

    if let Err(e) = ecrire() {
        let _ = std::fs::remove_file(&temporaire);
        return Err(erreur(e));
    }
    Ok(())
}

/// Le téléchargeur.
pub struct Telechargeur<'a> {
    client: &'a ClientHttp,
    /// La signature du substitut, s'il y en a un sur cette installation.
    signature: Option<SignaturePlaceholder>,
    /// Le contenu à écrire quand aucune source ne répond.
    substitut: Option<Vec<u8>>,
    /// Les images servies par la source de **repli** : `(destination, URL
    /// primaire)`. L'appelant les consigne, pour qu'une mise à jour de la
    /// base puisse retenter la source primaire.
    replis: std::sync::Mutex<Vec<(PathBuf, String)>>,
}

impl std::fmt::Debug for Telechargeur<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Telechargeur")
            .field("substitut", &self.substitut.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

impl<'a> Telechargeur<'a> {
    /// Prépare un téléchargeur.
    ///
    /// `reference` est le chemin de `notfound.jpg`. S'il existe, son contenu
    /// sert de substitut et sa signature permet de reconnaître les substituts
    /// déjà posés. S'il n'existe pas, **aucun substitut n'est écrit** : un
    /// échec laisse le fichier absent, et la carte sera retentée à la prochaine
    /// passe. C'est un écart assumé au Python, qui fabriquait une vignette unie
    /// à la volée — laquelle devenait indiscernable d'une vraie image et
    /// condamnait la carte pour de bon.
    #[must_use]
    pub fn nouveau(client: &'a ClientHttp, reference: &Path) -> Self {
        let substitut = std::fs::read(reference).ok();
        let signature = SignaturePlaceholder::lire(reference);
        Self {
            client,
            signature,
            substitut,
            replis: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Les images que la source de repli a servies depuis la création du
    /// téléchargeur : `(destination, URL primaire)`.
    #[must_use]
    pub fn replis(&self) -> Vec<(PathBuf, String)> {
        self.replis.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// La signature du substitut, pour le planificateur.
    #[must_use]
    pub fn signature(&self) -> Option<&SignaturePlaceholder> {
        self.signature.as_ref()
    }

    /// Ce fichier est-il un substitut posé par un échec précédent ?
    #[must_use]
    pub fn est_substitut(&self, chemin: &Path) -> bool {
        est_placeholder(chemin, self.signature.as_ref())
    }

    /// Télécharge une cible, avec repli puis substitut.
    ///
    /// Rend `true` si une des deux sources a répondu.
    pub async fn une(&self, cible: &Cible) -> bool {
        if self.tenter(&cible.url_primaire, &cible.destination).await {
            return true;
        }
        if let Some(repli) = &cible.url_repli {
            if self.tenter(repli, &cible.destination).await {
                if let Ok(mut v) = self.replis.lock() {
                    v.push((cible.destination.clone(), cible.url_primaire.clone()));
                }
                return true;
            }
        }
        // Dernier recours : poser le substitut, s'il y en a un.
        if let Some(octets) = &self.substitut {
            if let Err(e) = ecrire_atomique(&cible.destination, octets) {
                tracing::warn!(erreur = %e, "substitut non écrit");
            }
        }
        false
    }

    /// Une tentative sur une URL. N'écrit rien en cas d'échec.
    ///
    /// Le quota de l'hôte est attendu par `get_ok` lui-même, avant chaque
    /// envoi : l'attendre ici aussi coûtait deux créneaux par image
    /// (2,2 s chez Yugipedia au lieu de 1,1 — mesuré le 2026-10-01).
    async fn tenter(&self, url: &str, destination: &Path) -> bool {
        // Une seule requête Yugipedia en vol (API etiquette MediaWiki) : le
        // jeton est gardé jusqu'à la fin de la lecture du corps. Les autres
        // hôtes gardent le parallélisme.
        let _jeton = ygo_sources::http::en_serie(url).await;
        let reponse = match self.client.get_ok(url).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(url, erreur = %e, "téléchargement refusé");
                return false;
            }
        };

        let mut octets = Vec::new();
        let mut flux = reponse.bytes_stream();
        while let Some(morceau) = flux.next().await {
            match morceau {
                Ok(m) => octets.extend_from_slice(&m),
                Err(e) => {
                    tracing::debug!(url, erreur = %e, "flux interrompu");
                    return false;
                }
            }
        }

        // Le contrôle de taille vient AVANT l'écriture : une page d'erreur
        // rendue en 200 OK ne doit pas écraser un fichier valide.
        if (octets.len() as u64) < TAILLE_MINIMALE {
            tracing::debug!(url, taille = octets.len(), "réponse trop courte");
            return false;
        }

        match ecrire_atomique(destination, &octets) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(erreur = %e, "image non écrite");
                false
            }
        }
    }

    /// Télécharge toutes les cibles, [`PARALLELISME`] à la fois.
    ///
    /// `progression` est appelée après chaque image, avec le nombre traité et
    /// le total.
    pub async fn toutes<F>(&self, cibles: &[Cible], mut progression: F) -> Bilan
    where
        F: FnMut(usize, usize),
    {
        let total = cibles.len();
        let mut bilan = Bilan::default();
        let mut faites = 0;

        let mut flux = futures_util::stream::iter(cibles.iter().map(|c| self.une(c)))
            .buffer_unordered(PARALLELISME);

        while let Some(reussie) = flux.next().await {
            if reussie {
                bilan.reussies += 1;
            } else {
                bilan.echecs += 1;
            }
            faites += 1;
            progression(faites, total);
        }
        bilan
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn l_ecriture_atomique_ne_laisse_pas_de_fichier_temporaire() {
        let tmp = tempfile::tempdir().unwrap();
        let cible = tmp.path().join("sous").join("dossier").join("12345.jpg");
        ecrire_atomique(&cible, b"des octets d'image").unwrap();

        assert_eq!(std::fs::read(&cible).unwrap(), b"des octets d'image");
        let restes: Vec<_> = std::fs::read_dir(cible.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(restes, ["12345.jpg"], "aucun .part ne subsiste");
    }

    /// Le remplacement d'un fichier existant se fait d'un bloc : le contenu
    /// d'avant ou celui d'après, jamais un mélange.
    #[test]
    fn l_ecriture_atomique_remplace_un_fichier_existant() {
        let tmp = tempfile::tempdir().unwrap();
        let cible = tmp.path().join("12345.jpg");
        std::fs::write(&cible, b"ancien").unwrap();
        ecrire_atomique(&cible, b"nouveau").unwrap();
        assert_eq!(std::fs::read(&cible).unwrap(), b"nouveau");
    }

    /// Le nom du fichier temporaire dérive de la cible, extension comprise :
    /// deux images du même dossier ne peuvent pas se marcher dessus.
    #[test]
    fn deux_cibles_du_meme_dossier_ont_des_temporaires_distincts() {
        let tmp = tempfile::tempdir().unwrap();
        for nom in ["a.jpg", "b.jpg", "c.png"] {
            ecrire_atomique(&tmp.path().join(nom), b"contenu").unwrap();
        }
        let mut noms: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        noms.sort();
        assert_eq!(noms, ["a.jpg", "b.jpg", "c.png"]);
    }

    #[test]
    fn un_chemin_sans_extension_s_ecrit_aussi() {
        let tmp = tempfile::tempdir().unwrap();
        let cible = tmp.path().join("sans_extension");
        ecrire_atomique(&cible, b"x").unwrap();
        assert_eq!(std::fs::read(&cible).unwrap(), b"x");
    }

    #[test]
    fn le_parallelisme_et_la_taille_minimale_sont_ceux_du_python() {
        assert_eq!(PARALLELISME, 6);
        assert_eq!(TAILLE_MINIMALE, 200);
    }
}
