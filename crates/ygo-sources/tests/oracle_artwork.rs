// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le découpage des noms de fichiers Yugipedia, sur 1 613 cas réels.
//!
//! # D'où vient cet oracle
//!
//! La table `card_images_externes` de l'installation V1.0.4 contient **1 613
//! lignes**, chacune produite par le vrai `_decouper_fichier` sur un nom de
//! fichier que Yugipedia héberge réellement. Elle porte, pour chaque fichier,
//! la langue, la rareté, l'édition, la variante et l'identifiant synthétique
//! qu'en a tirés le Python.
//!
//! `outils/oracle_artwork.py` ne se contente pas de recopier cette table : il
//! **rejoue** les fonctions Python sur chaque nom de fichier et vérifie
//! qu'elles redonnent bien ce que la base porte. Une divergence signifierait
//! que la table a été écrite par une version antérieure du module, et l'oracle
//! serait faux. Le champ `desaccords` de la fixture enregistre le résultat de
//! ce contrôle ; ce test exige qu'il soit vide.
//!
//! # Ce que couvrent les 1 613 cas
//!
//! | dimension | valeurs rencontrées |
//! |---|---|
//! | sets | 9 — `RA02` (560), `LOCR` (302), `LDK2` (280), `RA05` (158)… |
//! | langues | `EN` (1 311), `JP` (302) |
//! | éditions | `1E` (1 098), aucune (302), `UE` (203), `LE` (10) |
//! | variantes | aucune (1 577), `EA` (20), `AA` (16) |
//! | raretés | 14 formes, dont `PScR`, `PlScR`, `QCScR`, `StR`, `GMR` — et `2` |
//!
//! Cette rareté `2` n'est pas une faute de saisie de ce commentaire : un nom de
//! fichier réel a un jeton numérique là où le format attend une abréviation, et
//! le découpage le prend pour une rareté. Le portage le reproduit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use serde::Deserialize;

use ygo_sources::yugipedia::artwork::{
    abbr_rarete, analyser_fichier, analyser_fichier_set, badge_depuis_uuid, cle_comparaison,
    decouper_fichier, est_extended_art, id_synthetique, langue_set_code, libelle_depuis_uuid,
    parser_variant_pool, prefixe_recherche, prefixe_set, score, InfosFichier,
};

#[derive(Deserialize)]
struct Fichiers {
    desaccords: Vec<serde_json::Value>,
    cas: Vec<CasFichier>,
}

#[derive(Deserialize)]
struct CasFichier {
    uuid: String,
    fichier: String,
    set_prefix: String,
    card_name: String,
    segment_nom: String,
    cle_carte: String,
    langue: String,
    rarete_abbr: String,
    edition: String,
    variante: String,
    image_id: i64,
    badge: String,
    libelle: String,
    est_extended_art: bool,
}

#[derive(Deserialize)]
struct Fonctions {
    noms: Vec<CasNom>,
    set_codes: Vec<CasSetCode>,
    raretes: Vec<CasRarete>,
    scores: Vec<CasScore>,
}

#[derive(Deserialize)]
struct CasNom {
    entree: String,
    cle: String,
    prefixe_recherche: String,
}

#[derive(Deserialize)]
struct CasSetCode {
    entree: String,
    prefixe_set: String,
    langue_set_code: String,
}

#[derive(Deserialize)]
struct CasRarete {
    entree: String,
    abbr: String,
}

#[derive(Deserialize)]
struct CasScore {
    infos: InfosOracle,
    langue_cible: String,
    abbr_cible: String,
    score: i64,
}

#[derive(Deserialize)]
struct InfosOracle {
    langue: String,
    rarete_abbr: String,
    edition: String,
    variante: String,
}

#[derive(Deserialize)]
struct CasVariantPool {
    fichier_wikitext: String,
    set_prefix: String,
    pool: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct Capture {
    wikitext: String,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn lire<T: serde::de::DeserializeOwned>(chemin: PathBuf) -> T {
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte)
        .unwrap_or_else(|e| panic!("fixture {} illisible : {e}", chemin.display()))
}

#[test]
fn le_decoupage_rust_reproduit_le_python_sur_1613_fichiers_reels() {
    let f: Fichiers = lire(fixtures().join("oracle/artwork/fichiers.json"));

    assert!(
        f.desaccords.is_empty(),
        "l'oracle lui-même est en désaccord avec le Python : {:?}",
        f.desaccords
    );
    assert!(
        f.cas.len() >= 1613,
        "l'oracle doit couvrir les 1 613 fichiers capturés, {} trouvés",
        f.cas.len()
    );

    for cas in &f.cas {
        let (segment, infos) = decouper_fichier(&cas.fichier, &cas.set_prefix)
            .unwrap_or_else(|| panic!("{} : découpage impossible", cas.fichier));

        assert_eq!(segment, cas.segment_nom, "{} : segment de nom", cas.fichier);
        assert_eq!(
            infos,
            InfosFichier {
                fichier: cas.fichier.clone(),
                langue: cas.langue.clone(),
                rarete_abbr: cas.rarete_abbr.clone(),
                edition: cas.edition.clone(),
                variante: cas.variante.clone(),
            },
            "{} : champs découpés",
            cas.fichier
        );
        assert_eq!(
            id_synthetique(&cas.fichier),
            cas.image_id,
            "{} : identifiant synthétique",
            cas.fichier
        );
        assert_eq!(
            cle_comparaison(&segment),
            cas.cle_carte,
            "{} : clé de carte",
            cas.fichier
        );
        assert_eq!(
            est_extended_art(&infos),
            cas.est_extended_art,
            "{} : extended art",
            cas.fichier
        );
        assert_eq!(
            badge_depuis_uuid(&cas.uuid),
            cas.badge,
            "{} : badge",
            cas.uuid
        );
        assert_eq!(
            libelle_depuis_uuid(&cas.uuid),
            cas.libelle,
            "{} : libellé",
            cas.uuid
        );
    }
}

#[test]
fn valider_et_extraire_le_nom_de_carte_sur_les_cas_reels() {
    let f: Fichiers = lire(fixtures().join("oracle/artwork/fichiers.json"));
    for cas in &f.cas {
        // La validation doit accepter le nom que l'application a stocké…
        assert!(
            analyser_fichier(&cas.fichier, &cas.card_name, &cas.set_prefix).is_some(),
            "{} : refusé pour {}",
            cas.fichier,
            cas.card_name
        );
        // …et refuser un autre nom.
        assert!(
            analyser_fichier(&cas.fichier, "Carte Qui N'Existe Pas", &cas.set_prefix).is_none(),
            "{} : accepté à tort",
            cas.fichier
        );
        // L'extraction doit rendre la même clé.
        let (segment, cle, _) = analyser_fichier_set(&cas.fichier, &cas.set_prefix)
            .unwrap_or_else(|| panic!("{} : extraction impossible", cas.fichier));
        assert_eq!(segment, cas.segment_nom);
        assert_eq!(cle, cas.cle_carte);
    }
}

#[test]
fn les_fonctions_pures_suivent_le_python() {
    let f: Fonctions = lire(fixtures().join("oracle/artwork/fonctions.json"));

    // 560 noms de cartes réels, plus une poignée de pièges : ponctuation,
    // tirets longs, deux-points, esperluette, accents, chaîne vide.
    assert!(f.noms.len() >= 560);
    for cas in &f.noms {
        assert_eq!(
            cle_comparaison(&cas.entree),
            cas.cle,
            "clé de {:?}",
            cas.entree
        );
        assert_eq!(
            prefixe_recherche(&cas.entree),
            cas.prefixe_recherche,
            "préfixe de recherche de {:?}",
            cas.entree
        );
    }

    for cas in &f.set_codes {
        assert_eq!(
            prefixe_set(&cas.entree),
            cas.prefixe_set,
            "{:?}",
            cas.entree
        );
        assert_eq!(
            langue_set_code(&cas.entree),
            cas.langue_set_code,
            "{:?}",
            cas.entree
        );
    }

    // Les 43 raretés du référentiel de l'utilisateur, plus six cas de casse.
    assert!(f.raretes.len() >= 49);
    for cas in &f.raretes {
        assert_eq!(
            abbr_rarete(&cas.entree),
            cas.abbr,
            "abbr de {:?}",
            cas.entree
        );
    }

    // 1 200 scores : chaque fichier réel face à trois cibles.
    assert!(f.scores.len() >= 1200);
    for cas in &f.scores {
        let infos = InfosFichier {
            fichier: String::new(),
            langue: cas.infos.langue.clone(),
            rarete_abbr: cas.infos.rarete_abbr.clone(),
            edition: cas.infos.edition.clone(),
            variante: cas.infos.variante.clone(),
        };
        assert_eq!(
            score(&infos, &cas.langue_cible, &cas.abbr_cible),
            cas.score,
            "score de {:?} vers ({}, {})",
            cas.infos.rarete_abbr,
            cas.langue_cible,
            cas.abbr_cible
        );
    }
}

#[test]
fn le_pool_de_variantes_suit_le_python_sur_le_wikitext_reel() {
    let racine = fixtures();
    let cas: Vec<CasVariantPool> = lire(racine.join("oracle/artwork/variant_pool.json"));
    assert_eq!(cas.len(), 2, "les deux pages capturées");

    for c in &cas {
        let capture: Capture = lire(racine.join("net/yugipedia").join(&c.fichier_wikitext));
        let obtenu = parser_variant_pool(&capture.wikitext, &c.set_prefix);
        let attendu: std::collections::BTreeMap<String, String> = c
            .pool
            .iter()
            .map(|p| (p[0].clone(), p[1].clone()))
            .collect();
        assert_eq!(obtenu, attendu, "{} : variant art pool", c.set_prefix);
        assert_eq!(obtenu.len(), 18, "{} : 18 cartes chase", c.set_prefix);
    }
}
