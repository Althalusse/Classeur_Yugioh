// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le repli YGOPRODeck, sur cinq sets réels.
//!
//! # Un oracle complet, entrée comprise
//!
//! Contrairement à celui de la construction locale, cet oracle porte **les deux
//! bouts** : la réponse de l'API telle qu'elle est arrivée
//! (`tests/fixtures/net/ygoprodeck/`) et les lignes que le Python en a tirées
//! (`tests/fixtures/oracle/api/`). Le test peut donc rejouer la transformation
//! entière et comparer **chaque champ**, là où l'oracle local devait se
//! contenter du tri.
//!
//! # Les cinq sets, et ce qu'ils exercent
//!
//! | set | lignes | ce qu'il apporte |
//! |---|---|---|
//! | `RA05` | 692 | le cas qui déclenche le repli — 692 contre 228 en local |
//! | `RA02` | 553 | sept raretés par carte, et **le même compte qu'en local** |
//! | `L26D` | 148 | multi-deck : groupes de lettres `ENM` / `ENS` / `ENX` |
//! | `LDK2` | 130 | le cas « 130 contre 132 » documenté dans le Python |
//! | `SDWD` | 49 | structure deck, une rareté par carte |
//!
//! # Ce que la capture ne garde pas, et pourquoi
//!
//! Les réponses brutes pèsent plusieurs mégaoctets — chaque carte porte son
//! texte d'effet et la liste de **tous** les sets où elle paraît. La capture ne
//! retient que la douzaine de champs que le transform consulte, et la réponse
//! française se réduit à `id` et `name`, les deux seuls qu'il en lit. La
//! réduction a été **vérifiée sans perte** : les lignes produites depuis les
//! captures réduites sont identiques, empreinte pour empreinte, à celles
//! produites depuis les réponses complètes.
//!
//! En revanche `card_sets` est conservé **entier** : c'est lui qui exerce le
//! filtrage par nom de set, sans quoi les tirages des autres sets où paraît la
//! carte se retrouveraient dans le classeur.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use serde::Deserialize;

use ygo_app::creation::{
    lignes_depuis_api, locales_semblent_incompletes, nom_du_set, CarteApi, LigneClasseur,
    SetYgoprodeck,
};

#[derive(Deserialize)]
struct Index {
    cas: Vec<CasIndex>,
}

#[derive(Deserialize)]
struct CasIndex {
    fichier: String,
    set_code: String,
    set_name: String,
    lignes: usize,
    cartes_uniques: usize,
    fichier_capture: String,
}

#[derive(Deserialize)]
struct Capture {
    set_code: String,
    set_name: String,
    en: Vec<CarteApi>,
    fr: Vec<CarteApi>,
}

#[derive(Deserialize)]
struct OracleApi {
    set_code: String,
    set_name: String,
    lignes: Vec<LigneOracle>,
}

/// La ligne telle que le Python la sérialise — mêmes clés que son dictionnaire.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
struct LigneOracle {
    card_uuid: String,
    card_image_uuid: String,
    card_image_id: Option<i64>,
    name: String,
    name_fr: String,
    set_code: String,
    rarity: String,
    rarity_code: String,
    set_name: String,
    card_image_url: String,
    card_image_small: String,
    sort_order: i64,
    card_type: String,
    atk: Option<i64>,
    def_val: Option<i64>,
    level: Option<i64>,
    attribute: Option<String>,
    race: String,
    extended_art: i64,
}

impl From<&LigneClasseur> for LigneOracle {
    fn from(l: &LigneClasseur) -> Self {
        Self {
            card_uuid: l.card_uuid.clone(),
            card_image_uuid: l.card_image_uuid.clone(),
            card_image_id: l.card_image_id,
            name: l.name.clone(),
            name_fr: l.name_fr.clone(),
            set_code: l.set_code.clone(),
            rarity: l.rarity.clone(),
            rarity_code: l.rarity_code.clone(),
            set_name: l.set_name.clone(),
            card_image_url: l.card_image_url.clone(),
            card_image_small: l.card_image_small.clone(),
            sort_order: l.sort_order,
            card_type: l.card_type.clone(),
            atk: l.atk,
            def_val: l.def_val,
            level: l.level,
            attribute: l.attribute.clone(),
            race: l.race.clone(),
            extended_art: l.extended_art,
        }
    }
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
fn le_transform_rust_reproduit_le_python_champ_par_champ() {
    let racine = fixtures();
    let index: Index = lire(racine.join("oracle/api/index.json"));
    assert_eq!(index.cas.len(), 5, "cinq sets capturés");

    let mut total = 0usize;
    for cas in &index.cas {
        let oracle: OracleApi = lire(racine.join("oracle/api").join(&cas.fichier));
        let capture: Capture = lire(racine.join("net/ygoprodeck").join(&cas.fichier_capture));
        let etiquette = &cas.set_code;

        // La capture doit bien être celle que l'oracle décrit.
        assert_eq!(capture.set_code, oracle.set_code, "{etiquette} : code");
        assert_eq!(
            capture.set_name, oracle.set_name,
            "{etiquette} : nom du set"
        );
        assert_eq!(cas.set_name, oracle.set_name);

        let obtenues = lignes_depuis_api(&capture.set_name, &capture.en, &capture.fr);
        assert_eq!(
            obtenues.len(),
            oracle.lignes.len(),
            "{etiquette} : {} lignes produites contre {} attendues",
            obtenues.len(),
            oracle.lignes.len()
        );
        assert_eq!(obtenues.len(), cas.lignes, "{etiquette} : index cohérent");

        for (i, (o, a)) in obtenues.iter().zip(&oracle.lignes).enumerate() {
            assert_eq!(
                &LigneOracle::from(o),
                a,
                "{etiquette} : divergence sur la ligne {i} ({} / {})",
                a.set_code,
                a.rarity
            );
        }
        total += obtenues.len();
    }
    assert_eq!(total, 1572, "148 + 130 + 553 + 692 + 49");
}

#[test]
fn le_nom_du_set_se_retrouve_dans_les_1032_sets() {
    let sets: Vec<SetYgoprodeck> = lire(fixtures().join("net/ygoprodeck/cardsets.json"));
    assert!(sets.len() >= 1000, "{} sets capturés", sets.len());

    assert_eq!(nom_du_set("RA05", &sets), Some("Rarity Collection 5"));
    assert_eq!(
        nom_du_set("ra05", &sets),
        Some("Rarity Collection 5"),
        "insensible à la casse"
    );
    assert_eq!(
        nom_du_set("LDK2", &sets),
        Some("Legendary Decks II"),
        "l'API n'accepte que le nom complet, jamais le préfixe"
    );
    assert_eq!(nom_du_set("CODEINEXISTANT", &sets), None);
    assert_eq!(
        nom_du_set("", &sets),
        None,
        "un code vide ne doit rien trouver"
    );
}

#[test]
fn les_tirages_des_autres_sets_sont_ecartes() {
    // Une carte de RA05 paraît dans dix sets ; seuls les tirages du set visé
    // doivent entrer dans le classeur.
    let racine = fixtures();
    let capture: Capture = lire(racine.join("net/ygoprodeck/RA05.json"));
    let lignes = lignes_depuis_api(&capture.set_name, &capture.en, &capture.fr);

    assert!(
        lignes.iter().all(|l| l.set_name == "Rarity Collection 5"),
        "toutes les lignes doivent appartenir au set visé"
    );
    assert!(
        lignes.iter().all(|l| l.set_code.starts_with("RA05-")),
        "et porter son préfixe"
    );

    // Le total des tirages disponibles est bien supérieur au total retenu.
    let disponibles: usize = capture.en.iter().map(|c| c.card_sets.len()).sum();
    assert!(
        disponibles > lignes.len() * 2,
        "{disponibles} tirages dans la réponse, {} retenus — le filtrage doit mordre",
        lignes.len()
    );
}

#[test]
fn le_repli_apporte_bien_plus_que_le_local_sur_ra05() {
    // C'est toute la raison d'être du repli : le local en donne 228, l'API 692.
    let racine = fixtures();
    let capture: Capture = lire(racine.join("net/ygoprodeck/RA05.json"));
    let api = lignes_depuis_api(&capture.set_name, &capture.en, &capture.fr);
    assert_eq!(api.len(), 692);

    let local: serde_json::Value = lire(racine.join("oracle/creation/V1.0.3_RA05.json"));
    let n_local = local["lignes"].as_array().map(Vec::len).unwrap_or(0);
    assert_eq!(n_local, 228);
    assert!(
        api.len() > n_local * 3,
        "le repli triple le contenu du classeur"
    );
}

#[test]
fn l_heuristique_d_incompletude_ne_veut_rien_dire_sur_des_lignes_d_api() {
    // Elle compte les `card_uuid` distincts — et l'API n'en fournit aucun.
    // Le Python ne l'applique d'ailleurs jamais aux lignes d'API : la mesurer
    // ici rend le piège visible plutôt que latent.
    let racine = fixtures();
    let index: Index = lire(racine.join("oracle/api/index.json"));
    for cas in &index.cas {
        assert_eq!(
            cas.cartes_uniques, 0,
            "{} : l'API ne rend pas d'identifiant YGOJSON",
            cas.set_code
        );
        let capture: Capture = lire(racine.join("net/ygoprodeck").join(&cas.fichier_capture));
        let lignes = lignes_depuis_api(&capture.set_name, &capture.en, &capture.fr);
        let verdict = locales_semblent_incompletes(&lignes);
        assert_eq!(verdict.cartes_uniques, 0);
        assert!(
            !verdict.suspect,
            "sans dénominateur, l'heuristique ne conclut pas"
        );
    }
}

#[test]
fn les_identifiants_ygojson_restent_vides_et_le_code_de_rarete_arrive() {
    // Les deux différences structurelles entre les deux chemins.
    let racine = fixtures();
    let capture: Capture = lire(racine.join("net/ygoprodeck/RA02.json"));
    let lignes = lignes_depuis_api(&capture.set_name, &capture.en, &capture.fr);

    assert!(lignes.iter().all(|l| l.card_uuid.is_empty()));
    assert!(lignes.iter().all(|l| l.card_image_uuid.is_empty()));
    // Le chemin local, lui, laisse `rarity_code` vide et remplit les uuid.
    assert!(
        lignes.iter().any(|l| !l.rarity_code.is_empty()),
        "l'API donne le code de rareté"
    );
    // Et les parenthèses ont été retirées.
    assert!(
        lignes
            .iter()
            .all(|l| !l.rarity_code.contains('(') && !l.rarity_code.contains(')')),
        "les parenthèses de `set_rarity_code` doivent tomber"
    );
}
