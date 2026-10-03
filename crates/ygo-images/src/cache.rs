// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Reconnaître un **substitut** posé par un téléchargement raté.
//!
//! Portage de `gestion_image_classeur.est_notfound_placeholder`.
//!
//! # Pourquoi il faut savoir les reconnaître
//!
//! Quand aucune source ne rend l'image, le téléchargeur écrit quand même un
//! fichier : `notfound.jpg` s'il existe, sinon une vignette unie. C'est un
//! contrat — le worker ne doit pas remettre indéfiniment la même carte en
//! file d'attente parce que son fichier manque.
//!
//! Mais un substitut n'est pas une image. Sans moyen de le distinguer, une
//! carte indisponible un jour le resterait **pour toujours** : le fichier
//! existe, donc plus personne ne redemande. La comparaison à la signature du
//! fichier de référence rouvre cette porte à chaque ouverture de classeur.
//!
//! # La comparaison est stricte, et c'est voulu
//!
//! Taille **et** empreinte MD5, contre le `notfound.jpg` de l'installation.
//! MD5 n'est pas là pour résister à quoi que ce soit — il compare deux
//! fichiers locaux dont personne n'a intérêt à forger la collision. Il est
//! rapide, et c'est ce que le Python employait.

use std::path::Path;

use md5::{Digest, Md5};

/// La signature du fichier de référence, lue une fois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignaturePlaceholder {
    /// Taille du fichier de référence, en octets.
    pub taille: u64,
    /// Empreinte MD5 du fichier de référence.
    pub empreinte: [u8; 16],
}

impl SignaturePlaceholder {
    /// Lit la signature de `notfound.jpg`.
    ///
    /// Rend `None` si le fichier de référence est absent — auquel cas plus
    /// rien n'est reconnu comme substitut, ce qui est exactement le
    /// comportement du Python (`if sig is None: return False`).
    #[must_use]
    pub fn lire(reference: &Path) -> Option<Self> {
        let octets = std::fs::read(reference).ok()?;
        Some(Self {
            taille: octets.len() as u64,
            empreinte: empreinte(&octets),
        })
    }
}

/// L'empreinte MD5 d'un contenu.
#[must_use]
pub fn empreinte(octets: &[u8]) -> [u8; 16] {
    let mut hacheur = Md5::new();
    hacheur.update(octets);
    hacheur.finalize().into()
}

/// Ce fichier est-il le substitut ?
///
/// La taille est comparée d'abord : elle écarte la quasi-totalité des cas sans
/// lire le fichier entier.
#[must_use]
pub fn est_placeholder(chemin: &Path, signature: Option<&SignaturePlaceholder>) -> bool {
    let Some(signature) = signature else {
        return false;
    };
    let Ok(meta) = std::fs::metadata(chemin) else {
        return false;
    };
    if meta.len() != signature.taille {
        return false;
    }
    let Ok(octets) = std::fs::read(chemin) else {
        return false;
    };
    empreinte(&octets) == signature.empreinte
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn le_substitut_se_reconnait_a_la_taille_et_a_l_empreinte() {
        let tmp = tempfile::tempdir().unwrap();
        let reference = tmp.path().join("notfound.jpg");
        std::fs::write(&reference, b"substitut de reference").unwrap();
        let signature = SignaturePlaceholder::lire(&reference).unwrap();

        let copie = tmp.path().join("carte.jpg");
        std::fs::write(&copie, b"substitut de reference").unwrap();
        assert!(est_placeholder(&copie, Some(&signature)));

        let vraie = tmp.path().join("vraie.jpg");
        std::fs::write(&vraie, b"une vraie image, bien plus longue").unwrap();
        assert!(!est_placeholder(&vraie, Some(&signature)));
    }

    /// Deux fichiers de **même taille** mais de contenu différent ne doivent
    /// pas se confondre : c'est ce que l'empreinte ajoute à la taille.
    #[test]
    fn deux_fichiers_de_meme_taille_ne_se_confondent_pas() {
        let tmp = tempfile::tempdir().unwrap();
        let reference = tmp.path().join("notfound.jpg");
        std::fs::write(&reference, b"AAAAAAAAAA").unwrap();
        let signature = SignaturePlaceholder::lire(&reference).unwrap();

        let autre = tmp.path().join("autre.jpg");
        std::fs::write(&autre, b"BBBBBBBBBB").unwrap();
        assert_eq!(std::fs::metadata(&autre).unwrap().len(), signature.taille);
        assert!(!est_placeholder(&autre, Some(&signature)));
    }

    /// Sans fichier de référence, plus rien n'est reconnu — et surtout, rien
    /// n'est reconnu **à tort**. Le Python fait pareil.
    #[test]
    fn sans_reference_rien_n_est_un_substitut() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            SignaturePlaceholder::lire(&tmp.path().join("absent.jpg")),
            None
        );
        let fichier = tmp.path().join("carte.jpg");
        std::fs::write(&fichier, b"x").unwrap();
        assert!(!est_placeholder(&fichier, None));
    }

    #[test]
    fn un_fichier_absent_n_est_pas_un_substitut() {
        let tmp = tempfile::tempdir().unwrap();
        let reference = tmp.path().join("notfound.jpg");
        std::fs::write(&reference, b"x").unwrap();
        let signature = SignaturePlaceholder::lire(&reference).unwrap();
        assert!(!est_placeholder(
            &tmp.path().join("absent.jpg"),
            Some(&signature)
        ));
    }
}
