// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Comparaison du tri Rust au tri de la V1.0.4 Python, sur les classeurs réels.
//!
//! Les fixtures de `tests/fixtures/oracle/tri/` ne sont pas écrites à la main :
//! elles sont produites par `outils/oracle_tri.py`, qui importe le vrai
//! `module/gestion_rarete/tri_carte.py` et fige, pour chaque classeur, l'entrée
//! exacte du tri et la sortie obtenue. **26 classeurs, 7 136 cartes**, tirés des
//! installations V1.0.3 (17 classeurs) et V1.0.4 (9 classeurs).
//!
//! Chaque fixture contient :
//!
//! - les cinq champs que le tri consulte, dans l'ordre où la V1.0.4 les lit
//!   (`ORDER BY sort_order, rarity`) — l'ordre d'entrée compte, le tri étant
//!   stable ;
//! - les priorités de rareté de l'installation concernée — les deux versions
//!   ne les numérotent pas pareil, ce qui fait deux jeux d'épreuves ;
//! - la permutation attendue pour **chacun des six ordres de critères** ;
//! - les indices conservés par le filtre N-raretés pour `n` = 1, 2, 3 et 5 ;
//! - le dictionnaire des rangs d'artwork.
//!
//! Un écart ici ne signale pas un test à ajuster : il signale que le portage
//! afficherait les cartes dans un ordre différent de celui que l'utilisateur
//! voit aujourd'hui.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use ygo_core::config::CritereTri;
use ygo_core::rarity::Priorites;
use ygo_core::tri::{indices_n_raretes_par_artwork, ordre_de_tri, rangs_artwork, Carte};

#[derive(Deserialize)]
struct Fixture {
    version: String,
    classeur: String,
    priorites: HashMap<String, i64>,
    cartes: Vec<CarteFixture>,
    art_ranks: Vec<RangFixture>,
    tri: HashMap<String, Vec<usize>>,
    filtre_n_raretes: HashMap<String, Vec<usize>>,
}

#[derive(Deserialize)]
struct CarteFixture {
    name: Option<String>,
    rarity: Option<String>,
    set_code: Option<String>,
    card_image_id: Option<i64>,
    extended_art: i64,
}

#[derive(Deserialize)]
struct RangFixture {
    name: String,
    set_code: String,
    extended_art: i64,
    card_image_id: i64,
    rang: usize,
}

#[derive(Deserialize)]
struct Index {
    fixtures: Vec<EntreeIndex>,
    total_cartes: usize,
}

#[derive(Deserialize)]
struct EntreeIndex {
    fichier: String,
    cartes: usize,
}

fn dossier() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/oracle/tri")
        .canonicalize()
        .expect("le dossier des fixtures d'oracle doit exister")
}

/// `NULL` en base devient chaîne vide côté Python (`c.get("name", "")` sur un
/// dictionnaire dont la valeur est `None` renvoie `None`… mais la requête ne
/// renvoie jamais `NULL` sur ces colonnes ; le repli est là par prudence).
fn carte(c: &CarteFixture) -> Carte {
    Carte {
        nom: c.name.clone().unwrap_or_default(),
        set_code: c.set_code.clone().unwrap_or_default(),
        rarete: c.rarity.clone().unwrap_or_default(),
        card_image_id: c.card_image_id.unwrap_or(0),
        extended_art: c.extended_art != 0,
    }
}

/// `"numero+artwork+rarete"` → `[Numero, Artwork, Rarete]`.
fn ordre_depuis_cle(cle: &str) -> [CritereTri; 3] {
    let v: Vec<CritereTri> = cle
        .split('+')
        .map(|c| CritereTri::depuis_code(c).expect("critère inconnu dans la fixture"))
        .collect();
    assert_eq!(v.len(), 3, "une fixture doit porter les trois critères");
    [v[0], v[1], v[2]]
}

fn charger(chemin: &std::path::Path) -> Fixture {
    let texte = std::fs::read_to_string(chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte)
        .unwrap_or_else(|e| panic!("fixture {} illisible : {e}", chemin.display()))
}

#[test]
fn le_tri_rust_reproduit_le_tri_python_sur_les_classeurs_reels() {
    let dossier = dossier();
    let index: Index = serde_json::from_str(
        &std::fs::read_to_string(dossier.join("index.json")).expect("index.json des fixtures"),
    )
    .expect("index.json illisible");

    // Garde-fou : sans cette borne, un dossier vidé par erreur ferait un test
    // vert qui ne vérifie rien.
    assert!(
        index.fixtures.len() >= 26,
        "l'oracle doit couvrir au moins les 26 classeurs capturés, {} trouvés",
        index.fixtures.len()
    );

    let mut cartes_vues = 0usize;
    let mut permutations_verifiees = 0usize;

    for entree in &index.fixtures {
        let f = charger(&dossier.join(&entree.fichier));
        let etiquette = format!("{}/{}", f.version, f.classeur);

        assert_eq!(
            f.cartes.len(),
            entree.cartes,
            "{etiquette} : l'index annonce {} cartes, la fixture en porte {}",
            entree.cartes,
            f.cartes.len()
        );

        let cartes: Vec<Carte> = f.cartes.iter().map(carte).collect();
        let priorites = Priorites::depuis_paires(f.priorites.iter().map(|(k, v)| (k.clone(), *v)));

        // ── Rangs d'artwork ────────────────────────────────────────────────
        let attendus: HashMap<(&str, &str, bool, i64), usize> = f
            .art_ranks
            .iter()
            .map(|r| {
                (
                    (
                        r.name.as_str(),
                        r.set_code.as_str(),
                        r.extended_art != 0,
                        r.card_image_id,
                    ),
                    r.rang,
                )
            })
            .collect();
        let rangs = rangs_artwork(&cartes);
        for (i, c) in cartes.iter().enumerate() {
            let cle = (
                c.nom.as_str(),
                c.set_code.as_str(),
                c.extended_art,
                c.card_image_id,
            );
            let attendu = attendus
                .get(&cle)
                .unwrap_or_else(|| panic!("{etiquette} : rang absent de l'oracle pour {cle:?}"));
            assert_eq!(
                rangs[i], *attendu,
                "{etiquette} : rang d'artwork de la carte {i} ({cle:?})"
            );
        }

        // ── Les six ordres de critères ─────────────────────────────────────
        assert_eq!(f.tri.len(), 6, "{etiquette} : six permutations attendues");
        for (cle, attendu) in &f.tri {
            let obtenu = ordre_de_tri(&cartes, ordre_depuis_cle(cle), &priorites);
            assert_eq!(
                &obtenu,
                attendu,
                "{etiquette} : ordre « {cle} » divergent\n{}",
                premier_ecart(&obtenu, attendu, &cartes)
            );
            permutations_verifiees += 1;
        }

        // ── Filtre N raretés par artwork ───────────────────────────────────
        for (n, attendu) in &f.filtre_n_raretes {
            let n: usize = n.parse().expect("n numérique dans la fixture");
            let obtenu = indices_n_raretes_par_artwork(&cartes, n, &priorites);
            assert_eq!(
                &obtenu,
                attendu,
                "{etiquette} : filtre n={n} divergent ({} cartes gardées contre {})",
                obtenu.len(),
                attendu.len()
            );
        }

        cartes_vues += cartes.len();
    }

    assert_eq!(
        cartes_vues, index.total_cartes,
        "toutes les cartes de l'oracle doivent être passées"
    );
    assert_eq!(
        permutations_verifiees,
        index.fixtures.len() * 6,
        "six permutations par classeur"
    );
}

/// Message d'échec utile : la première position qui diverge, et les deux cartes
/// qui s'y trouvent. Sans ça, l'échec affiche deux listes de plusieurs
/// centaines d'entiers.
fn premier_ecart(obtenu: &[usize], attendu: &[usize], cartes: &[Carte]) -> String {
    if obtenu.len() != attendu.len() {
        return format!(
            "longueurs différentes : {} contre {}",
            obtenu.len(),
            attendu.len()
        );
    }
    for (position, (&o, &a)) in obtenu.iter().zip(attendu).enumerate() {
        if o != a {
            let decrire = |i: usize| match cartes.get(i) {
                Some(c) => format!(
                    "#{i} {} / {} / {} / img {} / ext {}",
                    c.set_code, c.nom, c.rarete, c.card_image_id, c.extended_art
                ),
                None => format!("#{i} hors bornes"),
            };
            return format!(
                "  position {position}\n  Rust   : {}\n  Python : {}",
                decrire(o),
                decrire(a)
            );
        }
    }
    "aucun écart positionnel".to_owned()
}
