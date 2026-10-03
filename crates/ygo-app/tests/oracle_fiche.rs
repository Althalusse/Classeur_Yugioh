// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Oracle de la fiche de carte, sur des extraits des **vraies** bases.
//!
//! # Pourquoi trois classeurs et pas un
//!
//! Le `card_uuid` d'une ligne de classeur se retrouve par trois voies selon la
//! version qui a créé le classeur. Sur les neuf classeurs réels, 1 946 lignes :
//! 885 portent leur `card_uuid`, 927 se résolvent par `card_image_id`, 134 par
//! `set_code`. Un oracle qui ne prendrait qu'un classeur ne vérifierait qu'un
//! tiers du parc — et la voie non couverte pourrait être cassée sans que rien
//! ne le dise.
//!
//! `outils/extraire_oracle_fiche.py` choisit donc ses cartes **dans chaque
//! voie**, et recopie mécaniquement les lignes nécessaires : rien n'est
//! retranscrit à la main.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde::Deserialize;
use ygo_app::fiche;
use ygo_core::config::SourceImage;
use ygo_core::paths::Paths;

#[derive(Debug, Deserialize)]
struct TexteAttendu {
    langue: String,
    nom: Option<String>,
    effet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FicheAttendue {
    classeur: String,
    rowid: i64,
    resolution: String,
    set_code: String,
    nom: String,
    nom_fr: Option<String>,
    rarete: String,
    set_name: String,
    card_type: String,
    atk: Option<i64>,
    def: Option<i64>,
    level: Option<i64>,
    attribute: Option<String>,
    race: Option<String>,
    quantite: i64,
    qualite: Option<String>,
    edition: Option<String>,
    extended_art: i64,
    textes: Vec<TexteAttendu>,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/oracle/fiche")
}

fn attendues() -> Vec<FicheAttendue> {
    let chemin = fixtures().join("attendu.json");
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte).expect("attendu.json illisible")
}

/// Une installation jetable, disposée comme la vraie.
///
/// Les fixtures sont à plat ; `Paths` attend `bdd/cardinfo.db` et
/// `bdd/classeur_creer/<CODE>/<CODE>.db`. On recopie plutôt que de tordre
/// `Paths` pour les tests : c'est le vrai chemin qui doit être exercé.
fn installation(codes: &[&str]) -> (tempfile::TempDir, Paths) {
    let tmp = tempfile::tempdir().expect("dossier temporaire");
    let racine = tmp.path().to_path_buf();
    let paths = Paths::depuis_racine(&racine);
    std::fs::create_dir_all(paths.classeurs()).unwrap();
    copier(&fixtures().join("cardinfo.db"), &paths.cardinfo_db());
    for code in codes {
        std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
        copier(
            &fixtures().join(format!("{code}.db")),
            &paths.classeur_db(code),
        );
    }
    (tmp, paths)
}

fn copier(de: &Path, vers: &Path) {
    std::fs::copy(de, vers)
        .unwrap_or_else(|e| panic!("copie {} → {} : {e}", de.display(), vers.display()));
}

/// Chaque fiche rend exactement ce que les vraies bases contiennent.
#[test]
fn la_fiche_rend_ce_que_la_base_contient() {
    let attendues = attendues();
    assert_eq!(attendues.len(), 7, "sept cartes figées");

    let codes: Vec<&str> = ["LOCR-JP", "EGO1", "RA05"].into();
    let (_tmp, paths) = installation(&codes);

    for a in &attendues {
        let f = fiche::lire(&paths, &a.classeur, a.rowid, SourceImage::default())
            .unwrap_or_else(|e| panic!("{} rowid {} : {e}", a.classeur, a.rowid))
            .unwrap_or_else(|| panic!("{} rowid {} introuvable", a.classeur, a.rowid));
        let ou = format!("{} rowid {} ({})", a.classeur, a.rowid, a.set_code);

        assert_eq!(
            f.resolution.mot(),
            a.resolution,
            "voie de résolution — {ou}"
        );
        assert_eq!(f.set_code, a.set_code, "{ou}");
        assert_eq!(f.nom, a.nom, "{ou}");
        assert_eq!(f.nom_fr, a.nom_fr.clone().unwrap_or_default(), "{ou}");
        assert_eq!(f.rarete, a.rarete, "{ou}");
        assert_eq!(f.set_name, a.set_name, "{ou}");
        assert_eq!(f.card_type, a.card_type, "{ou}");
        assert_eq!(f.atk, a.atk, "{ou}");
        assert_eq!(f.def, a.def, "{ou}");
        assert_eq!(f.level, a.level, "{ou}");
        assert_eq!(f.attribute, a.attribute.clone().unwrap_or_default(), "{ou}");
        assert_eq!(f.race, a.race.clone().unwrap_or_default(), "{ou}");
        assert_eq!(f.quantite, a.quantite, "{ou}");
        assert_eq!(f.qualite, a.qualite, "{ou}");
        assert_eq!(f.edition, a.edition, "{ou}");
        assert_eq!(f.extended_art, a.extended_art != 0, "{ou}");

        // Les textes, langue par langue, dans l'ordre de la base.
        let rendus: Vec<(&str, Option<&str>, Option<&str>)> = f
            .textes
            .iter()
            .map(|t| (t.langue.as_str(), t.nom.as_deref(), t.effet.as_deref()))
            .collect();
        let voulus: Vec<(&str, Option<&str>, Option<&str>)> = a
            .textes
            .iter()
            .map(|t| (t.langue.as_str(), t.nom.as_deref(), t.effet.as_deref()))
            .collect();
        assert_eq!(rendus, voulus, "textes — {ou}");
    }
}

/// Les trois voies de résolution sont bien toutes exercées.
///
/// Sans cette vérification, une fixture régénérée qui perdrait une voie
/// laisserait le test vert tout en ne prouvant plus qu'un tiers de ce qu'il
/// prétend prouver.
#[test]
fn l_oracle_couvre_les_trois_voies() {
    let mut vues: Vec<String> = attendues().into_iter().map(|a| a.resolution).collect();
    vues.sort();
    vues.dedup();
    assert_eq!(vues, vec!["code", "image", "uuid"]);
}

/// Le repli de langue, sur le cas réel qui l'exige.
///
/// `LOCR-JP001` n'a que `en`, `ja` et `zh-CN` : c'est une carte d'un set
/// japonais récent, le français n'existe pas. Une fiche réglée en français
/// doit montrer l'anglais plutôt que rien.
#[test]
fn une_carte_sans_texte_francais_se_replie_sur_l_anglais() {
    let (_tmp, paths) = installation(&["LOCR-JP"]);
    let f = fiche::lire(&paths, "LOCR-JP", 1, SourceImage::default())
        .unwrap()
        .unwrap();

    assert!(
        f.texte_en("fr").is_none(),
        "cette carte n'a pas de texte français : {:?}",
        f.langues("fr")
    );
    let t = f.texte("fr").expect("un texte quand même");
    assert_eq!(t.langue, "en", "l'anglais prend le relais");
    assert!(
        t.effet.as_deref().is_some_and(|e| !e.trim().is_empty()),
        "et il n'est pas vide"
    );
    // Le chinois est là — nom traduit, effet vide — et n'est donc pas offert.
    // La base distingue « traduit » de « nommé » ; la fiche aussi, sans quoi
    // elle proposerait un onglet qui n'affiche rien.
    assert!(
        f.textes
            .iter()
            .any(|t| t.langue == "zh-CN" && !t.utilisable()),
        "la ligne zh-CN existe mais son effet est vide"
    );
    assert_eq!(f.langues("fr"), vec!["en", "ja"]);
}

/// Une carte complète, elle, sort en français.
#[test]
fn une_carte_traduite_sort_en_francais() {
    let (_tmp, paths) = installation(&["EGO1"]);
    let f = fiche::lire(&paths, "EGO1", 1, SourceImage::default())
        .unwrap()
        .unwrap();

    assert_eq!(f.langues("fr").first().map(String::as_str), Some("fr"));
    let t = f.texte("fr").expect("texte français");
    assert_eq!(t.langue, "fr");
    assert!(
        t.effet.as_deref().is_some_and(|e| e.len() > 40),
        "un effet, pas un fragment"
    );
    // La voie de résolution de ce classeur : il n'a pas de `card_uuid`.
    assert_eq!(f.resolution, fiche::Resolution::ParImage);
}

/// Sans `cardinfo.db`, la fiche existe encore — sans texte.
///
/// Une installation neuve, ou un classeur isolé, ne doit pas faire échouer
/// l'ouverture : le nom, la rareté et l'illustration valent mieux que rien.
#[test]
fn sans_base_des_cartes_la_fiche_garde_ce_que_le_classeur_sait() {
    let (_tmp, paths) = installation(&["EGO1"]);
    std::fs::remove_file(paths.cardinfo_db()).unwrap();

    let f = fiche::lire(&paths, "EGO1", 1, SourceImage::default())
        .expect("pas d'erreur")
        .expect("la ligne existe toujours");
    assert!(!f.nom.is_empty());
    assert!(f.textes.is_empty(), "aucun texte, et c'est normal");
    assert_eq!(f.resolution, fiche::Resolution::Aucune);
    assert!(f.texte("fr").is_none());
}

/// Une ligne qui n'existe pas se dit par `None`.
#[test]
fn une_ligne_absente_ne_leve_pas_d_erreur() {
    let (_tmp, paths) = installation(&["EGO1"]);
    assert!(fiche::lire(&paths, "EGO1", 9_999, SourceImage::default())
        .unwrap()
        .is_none());
}
