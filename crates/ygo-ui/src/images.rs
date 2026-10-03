// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le cache de textures des cartes.
//!
//! Portage du `_CTK_IMAGE_CACHE` du Python, et de sa raison d'être : le
//! redimensionnement et les filtres couleur étaient refaits **à chaque rendu
//! de page**, ce qui donnait un à-coup visible au changement de double page.
//!
//! # Ce qui compose la clé
//!
//! `(fichier, possédée, survolée)`. La **taille n'y est pas**, contrairement
//! au Python : egui redimensionne la texture au dessin, alors que Pillow devait
//! produire une image par taille. Une seule texture par carte et par état
//! suffit donc, et le redimensionnement de fenêtre n'invalide plus rien.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::attenuation::{attenuer, Facteurs};

/// Ce qui identifie une texture.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cle {
    /// Nom du fichier dans le cache d'images.
    pub fichier: String,
    /// La carte est-elle possédée ?
    pub possedee: bool,
    /// Est-elle survolée ?
    pub survolee: bool,
}

impl Cle {
    /// La clé d'une carte dans un état donné.
    #[must_use]
    pub fn nouvelle(fichier: &str, possedee: bool, survolee: bool) -> Self {
        Self {
            fichier: fichier.to_owned(),
            // Une carte possédée s'affiche pareil, survolée ou non : les
            // distinguer doublerait le cache pour rien.
            survolee: survolee && !possedee,
            possedee,
        }
    }
}

/// Les textures déjà préparées.
#[derive(Default)]
pub struct Cache {
    textures: HashMap<Cle, egui::TextureHandle>,
    /// Fichiers dont la lecture a échoué — on ne réessaie pas à chaque image.
    manquants: std::collections::HashSet<String>,
    /// Nombre de décodages réellement effectués, pour la mesure.
    decodages: usize,
    /// Temps cumulé passé à décoder.
    ///
    /// La mesure qui manquait : le nombre de décodages ne dit pas ce qu'ils
    /// coûtent, et c'est leur coût qui se voyait au changement de page.
    duree_decodage: std::time::Duration,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("textures", &self.textures.len())
            .field("manquants", &self.manquants.len())
            .field("decodages", &self.decodages)
            .field("duree_decodage", &self.duree_decodage)
            .finish()
    }
}

impl Cache {
    /// Combien de textures sont en mémoire.
    #[must_use]
    pub fn taille(&self) -> usize {
        self.textures.len()
    }

    /// Combien d'images ont réellement été décodées depuis le début.
    ///
    /// C'est la mesure qui compte : elle doit se stabiliser quand on feuillette
    /// en arrière, sans quoi le cache ne sert à rien.
    #[must_use]
    pub fn decodages(&self) -> usize {
        self.decodages
    }

    /// Combien de fichiers manquent au cache disque.
    #[must_use]
    pub fn manquants(&self) -> usize {
        self.manquants.len()
    }

    /// Temps cumulé passé à décoder depuis le début.
    ///
    /// Neuf cartes par page, décodées **dans la frame** : c'est ce cumul,
    /// rapporté au nombre de décodages, qui dit si un changement de page
    /// coûte un battement de cil ou deux secondes et demie.
    #[must_use]
    pub fn duree_decodage(&self) -> std::time::Duration {
        self.duree_decodage
    }

    /// Vide le cache — après un changement de source d'images, par exemple.
    pub fn vider(&mut self) {
        self.textures.clear();
        self.manquants.clear();
    }

    /// Oublie les fichiers réputés absents, **sans toucher aux textures**.
    ///
    /// C'est ce qu'il faut appeler quand une passe de téléchargement vient
    /// d'écrire de nouveaux fichiers : les cartes qui montraient un
    /// emplacement vide doivent retenter leur lecture, mais les dizaines de
    /// textures déjà décodées n'ont aucune raison de l'être une seconde fois.
    ///
    /// [`vider`](Self::vider) ferait aussi le travail, en repayant tout le
    /// décodage de la page courante à chaque image reçue.
    pub fn oublier_manquants(&mut self) {
        self.manquants.clear();
    }

    /// La texture d'une carte, décodée et atténuée si nécessaire.
    ///
    /// Rend `None` quand le fichier n'est pas dans le cache disque : l'appelant
    /// dessine alors un emplacement vide. C'est le cas courant — 20 % des
    /// cartes de `RA05` n'ont pas encore leur image.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        dossier: &Path,
        cle: Cle,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(&cle) {
            return Some(texture.clone());
        }
        if self.manquants.contains(&cle.fichier) {
            return None;
        }

        let chemin: PathBuf = dossier.join(&cle.fichier);
        let depart = std::time::Instant::now();
        let Some(image) = decoder(&chemin) else {
            self.manquants.insert(cle.fichier.clone());
            return None;
        };
        self.duree_decodage += depart.elapsed();
        self.decodages += 1;

        let mut pixels = image.pixels;
        attenuer(&mut pixels, Facteurs::pour(cle.possedee, cle.survolee));
        let couleurs =
            egui::ColorImage::from_rgba_unmultiplied([image.largeur, image.hauteur], &pixels);
        let texture = ctx.load_texture(
            format!("carte/{}/{}", cle.fichier, u8::from(cle.possedee)),
            couleurs,
            egui::TextureOptions::LINEAR,
        );
        self.textures.insert(cle, texture.clone());
        Some(texture)
    }
}

/// Une image décodée, en RGBA.
struct Decodee {
    pixels: Vec<u8>,
    largeur: usize,
    hauteur: usize,
}

/// Décode un fichier image, ou rend `None` s'il est absent ou illisible.
fn decoder(chemin: &Path) -> Option<Decodee> {
    let octets = std::fs::read(chemin).ok()?;
    let image = image::load_from_memory(&octets).ok()?.into_rgba8();
    let (largeur, hauteur) = image.dimensions();
    Some(Decodee {
        pixels: image.into_raw(),
        largeur: largeur as usize,
        hauteur: hauteur as usize,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Écrit un PNG minuscule mais **valide** : le cache ne doit pas être
    /// testé contre un fichier que le décodeur refuserait pour une autre
    /// raison que son absence.
    fn ecrire_png(chemin: &Path) {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 40, 40, 255]));
        image
            .save_with_format(chemin, image::ImageFormat::Png)
            .unwrap();
    }

    #[test]
    fn une_carte_possedee_ignore_le_survol_dans_la_cle() {
        // Elle s'affiche pareil dans les deux cas : deux entrées de cache
        // pour la même image seraient du gaspillage pur.
        let repos = Cle::nouvelle("x.jpg", true, false);
        let survol = Cle::nouvelle("x.jpg", true, true);
        assert_eq!(repos, survol);
    }

    #[test]
    fn une_carte_absente_distingue_le_survol() {
        let repos = Cle::nouvelle("x.jpg", false, false);
        let survol = Cle::nouvelle("x.jpg", false, true);
        assert_ne!(repos, survol, "elle s'éclaircit au survol");
    }

    #[test]
    fn deux_cartes_partageant_un_fichier_mais_pas_l_etat_different() {
        assert_ne!(
            Cle::nouvelle("x.jpg", true, false),
            Cle::nouvelle("x.jpg", false, false)
        );
    }

    #[test]
    fn un_fichier_absent_n_est_signale_qu_une_fois() {
        // On ne veut pas tenter d'ouvrir un fichier manquant à chaque image
        // dessinée — 60 fois par seconde et par carte.
        let dossier = tempfile::tempdir().unwrap();
        assert!(decoder(&dossier.path().join("jamais-vu.jpg")).is_none());
    }

    #[test]
    fn le_cache_part_vide() {
        let cache = Cache::default();
        assert_eq!(cache.taille(), 0);
        assert_eq!(cache.decodages(), 0);
        assert_eq!(cache.manquants(), 0);
    }

    /// Oublier les manquants doit rendre leur chance aux fichiers absents,
    /// **sans** jeter les textures déjà décodées — sinon chaque image reçue
    /// ferait repayer le décodage de toute la page.
    #[test]
    fn oublier_les_manquants_garde_les_textures() {
        let ctx = egui::Context::default();
        let tmp = tempfile::tempdir().unwrap();
        ecrire_png(&tmp.path().join("presente.jpg"));

        let mut cache = Cache::default();
        assert!(cache
            .texture(&ctx, tmp.path(), Cle::nouvelle("presente.jpg", true, false))
            .is_some());
        assert!(cache
            .texture(&ctx, tmp.path(), Cle::nouvelle("absente.jpg", true, false))
            .is_none());
        assert_eq!(cache.taille(), 1);
        assert_eq!(cache.manquants(), 1);
        let decodages = cache.decodages();

        cache.oublier_manquants();
        assert_eq!(cache.manquants(), 0, "l'absente sera retentée");
        assert_eq!(cache.taille(), 1, "la texture décodée est gardée");

        // Et la relire ne coûte aucun décodage supplémentaire.
        assert!(cache
            .texture(&ctx, tmp.path(), Cle::nouvelle("presente.jpg", true, false))
            .is_some());
        assert_eq!(cache.decodages(), decodages);
    }
}
