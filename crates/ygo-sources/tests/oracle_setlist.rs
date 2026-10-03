// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le parser de Set lists, comparé au Python sur le wikitext réel.
//!
//! `outils/oracle_overframe.py` fige deux choses côte à côte : le wikitext tel
//! que Yugipedia l'a rendu (`tests/fixtures/net/yugipedia/`), et ce que le
//! **vrai `sync_reference.parse_set_list`** en tire
//! (`tests/fixtures/oracle/overframe/`). Ce test rejoue le parser Rust sur le
//! premier et exige le second, entrée par entrée, champ par champ.
//!
//! # Ces révisions sont celles des vraies constructions de base
//!
//! | page | révision | vue par |
//! |---|---|---|
//! | `Set Card Lists:Limit Over Collection: The Rivals (OCG-JP)` | **5945579** | la V1.0.4, le 2026-08-25 |
//! | `Set Card Lists:Limit Over Collection: The Heroes (OCG-JP)` | **5892606** | la V1.0.3, le 2026-06-01 |
//!
//! Yugipedia ayant refusé `action=parse&oldid=` (403), la capture est passée
//! par la révision courante — qui s'est trouvée être exactement celle-là, les
//! deux pages n'ayant pas bougé depuis. Le `revid` rendu par l'API et
//! enregistré dans la fixture le prouve, et le test le vérifie : si une
//! recapture rendait une autre révision, l'assertion tomberait au lieu de
//! comparer silencieusement à autre chose.
//!
//! # Ce que le wikitext réel contient et que l'on n'aurait pas inventé
//!
//! ```text
//! {{Set page header}}
//!
//! {{Set list|region=JP|print=Reprint|
//! LOCR-JP001; Blue-Eyes White Dragon…; Ultra Rare, Secret Rare; New
//! LOCR-JP001; Blue-Eyes White Dragon…; Ultra Rare, Prismatic Secret Rare,
//!             Grand Master Rare; New // description::(extended art)
//! ```
//!
//! Un modèle précède le bloc — le découpage doit l'ignorer. Une **quatrième
//! colonne** (`New`) suit les raretés — elle ne fait pas partie du format
//! documenté, et un parser qui découperait sur un nombre de champs fixe s'y
//! casserait. Il n'y a **pas** de paramètre `rarities=` dans l'en-tête de ces
//! deux pages : l'héritage n'est donc pas exercé ici, il l'est par les tests
//! unitaires du module.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use serde::Deserialize;

use ygo_sources::yugipedia::{extraire_blocs, parse_set_list, EntreeSetList};

#[derive(Deserialize)]
struct Capture {
    prefix: String,
    titre: String,
    revid: String,
    wikitext: String,
}

#[derive(Deserialize)]
struct OracleParse {
    prefix: String,
    revid: String,
    titre: String,
    fichier_wikitext: String,
    blocs_set_list: usize,
    entrees: Vec<EntreeSetList>,
    normal_set: Vec<Vec<String>>,
    ext_set: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct Index {
    parsing: Vec<EntreeIndex>,
}

#[derive(Deserialize)]
struct EntreeIndex {
    fichier: String,
    entrees: usize,
    normal: usize,
    ext: usize,
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
fn le_parser_rust_reproduit_le_parser_python_sur_le_wikitext_reel() {
    let racine = fixtures();
    let index: Index = lire(racine.join("oracle/overframe/index.json"));
    assert_eq!(
        index.parsing.len(),
        2,
        "l'oracle doit couvrir les deux pages capturées"
    );

    for entree in &index.parsing {
        let oracle: OracleParse = lire(racine.join("oracle/overframe").join(&entree.fichier));
        let capture: Capture = lire(racine.join("net/yugipedia").join(&oracle.fichier_wikitext));
        let etiquette = format!("{}@{}", oracle.prefix, oracle.revid);

        // La capture doit bien être celle que l'oracle décrit — sans quoi on
        // comparerait le parsing d'une page à la sortie attendue d'une autre.
        assert_eq!(capture.prefix, oracle.prefix, "{etiquette} : préfixe");
        assert_eq!(capture.revid, oracle.revid, "{etiquette} : révision");
        assert_eq!(capture.titre, oracle.titre, "{etiquette} : titre de page");

        assert_eq!(
            extraire_blocs(&capture.wikitext).len(),
            oracle.blocs_set_list,
            "{etiquette} : nombre de blocs {{{{Set list}}}}"
        );

        let obtenues = parse_set_list(&capture.wikitext);
        assert_eq!(
            obtenues.len(),
            oracle.entrees.len(),
            "{etiquette} : {} entrées lues contre {} attendues",
            obtenues.len(),
            oracle.entrees.len()
        );
        assert_eq!(
            obtenues.len(),
            entree.entrees,
            "{etiquette} : index cohérent"
        );

        for (i, (o, a)) in obtenues.iter().zip(&oracle.entrees).enumerate() {
            assert_eq!(o, a, "{etiquette} : divergence sur l'entrée {i}");
        }
    }
}

/// Le classement `(numéro, rareté)` déduit des entrées, comparé au Python.
///
/// La fonction vit dans `ygo-app` ; on la réimplémente ici en trois lignes
/// plutôt que d'ajouter une dépendance de test entre crates — `ygo-app`
/// dépend de `ygo-sources`, l'inverse tournerait en rond. La vraie est
/// éprouvée par `oracle_overframe`, qui l'enchaîne à la réconciliation.
#[test]
fn les_entrees_lues_portent_les_memes_raretes_que_chez_python() {
    let racine = fixtures();
    let index: Index = lire(racine.join("oracle/overframe/index.json"));

    for entree in &index.parsing {
        let oracle: OracleParse = lire(racine.join("oracle/overframe").join(&entree.fichier));
        let capture: Capture = lire(racine.join("net/yugipedia").join(&oracle.fichier_wikitext));
        let etiquette = format!("{}@{}", oracle.prefix, oracle.revid);

        let obtenues = parse_set_list(&capture.wikitext);

        // Tous les couples (numéro, rareté), sans distinction de cadre : la
        // réunion des deux ensembles du Python.
        let mut attendus: Vec<(String, String)> = oracle
            .normal_set
            .iter()
            .chain(&oracle.ext_set)
            .map(|p| (p[0].clone(), p[1].clone()))
            .collect();
        attendus.sort();
        attendus.dedup();

        let mut couples: Vec<(String, String)> = obtenues
            .iter()
            .flat_map(|e| {
                e.raretes
                    .iter()
                    .map(|r| (e.numero.clone(), r.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        couples.sort();
        couples.dedup();

        assert_eq!(couples, attendus, "{etiquette} : couples (numéro, rareté)");
        assert_eq!(
            oracle.normal_set.len() + oracle.ext_set.len(),
            entree.normal + entree.ext,
            "{etiquette} : index cohérent"
        );
    }
}
