// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le tri de création de classeur, sur les 26 classeurs réels.
//!
//! # Ce que l'oracle contient, et pourquoi si peu
//!
//! `outils/oracle_creation.py` appelle le vrai `_build_rows_from_local_db` sur
//! les deux installations et fige sa sortie — **5 488 lignes** réparties sur
//! 26 classeurs. Mais il ne fige que cinq champs par ligne : `set_code`,
//! `rarity`, `extended_art`, `sort_order` et `card_uuid`.
//!
//! Ce n'est pas de l'économie de place, c'est de l'honnêteté. Les autres champs
//! — URLs, noms, statistiques — sortent tout droit des jointures SQL, et un
//! test qui les rejouerait aurait besoin de la base d'entrée, pas de la sortie.
//! Les figer aurait triplé le poids de la fixture sans que rien ne les
//! vérifie. Ils sont éprouvés autrement, par les bases miniatures des tests
//! unitaires du module.
//!
//! Les cinq champs conservés sont exactement ceux que consomment les fonctions
//! pures : la clé de tri, le cadre, la rareté, le rang produit, et le
//! `card_uuid` dont dépend l'heuristique d'incomplétude.
//!
//! # Les ex æquo, et comment on les traite
//!
//! Sur 5 488 lignes, **71** partagent leur clé de tri complète avec une autre :
//! même numéro, même cadre, même rareté, deux illustrations différentes. Leur
//! ordre relatif vient alors du parcours SQLite, que la fixture ne capture pas.
//!
//! Le test n'exige donc pas la permutation exacte — il exigerait quelque chose
//! que le Python lui-même ne garantit pas. Il exige que la **suite des clés**
//! soit identique, et que l'ensemble des lignes soit préservé. Un comparateur
//! fautif casse l'un ou l'autre.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use serde::Deserialize;

use ygo_app::creation::{
    cle_tri, locales_semblent_incompletes, ordre_depuis_code, trier_et_numeroter, LigneClasseur,
    SEUIL_AVG_RARETES_SUSPECT,
};

#[derive(Deserialize)]
struct Index {
    cas: Vec<CasIndex>,
    total_lignes: usize,
}

#[derive(Deserialize)]
struct CasIndex {
    #[serde(default)]
    fichier: Option<String>,
    #[serde(default)]
    identique_a: Option<String>,
    version: String,
    classeur: String,
    lignes: usize,
    suspect: bool,
    moyenne: f64,
    cartes_uniques: usize,
}

#[derive(Deserialize)]
struct Fixture {
    classeur: String,
    incompletude: Incompletude,
    lignes: Vec<LigneOracle>,
}

#[derive(Deserialize)]
struct Incompletude {
    suspect: bool,
    moyenne: f64,
    cartes_uniques: usize,
    seuil: f64,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
struct LigneOracle {
    set_code: String,
    rarity: String,
    extended_art: i64,
    sort_order: i64,
    card_uuid: String,
}

fn dossier() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/oracle/creation")
}

fn lire<T: serde::de::DeserializeOwned>(chemin: PathBuf) -> T {
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte)
        .unwrap_or_else(|e| panic!("fixture {} illisible : {e}", chemin.display()))
}

fn en_lignes(oracle: &[LigneOracle]) -> Vec<LigneClasseur> {
    oracle
        .iter()
        .map(|o| LigneClasseur {
            set_code: o.set_code.clone(),
            rarity: o.rarity.clone(),
            extended_art: o.extended_art,
            card_uuid: o.card_uuid.clone(),
            ..LigneClasseur::default()
        })
        .collect()
}

/// Mélange déterministe — un générateur congruentiel suffit, on ne cherche pas
/// du hasard mais un ordre d'entrée qui ne soit pas déjà le bon.
fn melanger<T>(v: &mut [T]) {
    let mut graine: u64 = 0x2545_F491_4F6C_DD1D;
    for i in (1..v.len()).rev() {
        graine = graine
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        v.swap(i, (graine >> 33) as usize % (i + 1));
    }
}

fn cles(lignes: &[LigneClasseur]) -> Vec<((&str, i64), i64, &str)> {
    lignes
        .iter()
        .map(|l| {
            (
                cle_tri(&l.set_code),
                i64::from(l.extended_art != 0),
                l.rarity.as_str(),
            )
        })
        .collect()
}

#[test]
fn le_tri_rust_reproduit_l_ordre_du_python_sur_5488_lignes() {
    let dossier = dossier();
    let index: Index = lire(dossier.join("index.json"));
    assert_eq!(index.cas.len(), 26, "26 classeurs capturés");
    assert_eq!(index.total_lignes, 5488);

    let mut vues = 0usize;
    for cas in &index.cas {
        let Some(fichier) = &cas.fichier else {
            // Sortie identique à une autre déjà figée : rien à rejouer ici,
            // c'est le test suivant qui vérifie que le fait est documenté.
            assert!(cas.identique_a.is_some());
            continue;
        };
        let f: Fixture = lire(dossier.join(fichier));
        let etiquette = format!("{}/{}", cas.version, f.classeur);
        assert_eq!(f.lignes.len(), cas.lignes, "{etiquette} : index cohérent");

        let attendues = en_lignes(&f.lignes);

        // La fixture est déjà dans l'ordre de sortie du Python : la retrier
        // depuis un ordre quelconque doit redonner la même suite de clés.
        let mut obtenues = attendues.clone();
        melanger(&mut obtenues);
        trier_et_numeroter(&mut obtenues);

        assert_eq!(
            cles(&obtenues),
            cles(&attendues),
            "{etiquette} : suite des clés de tri"
        );

        // Rien n'a été perdu ni inventé — les ex æquo peuvent avoir permuté,
        // l'ensemble non.
        let mut a: Vec<_> = obtenues.iter().map(|l| &l.card_uuid).collect();
        let mut b: Vec<_> = attendues.iter().map(|l| &l.card_uuid).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "{etiquette} : ensemble des cartes");

        // Le rang est la position, sans trou ni doublon.
        assert_eq!(
            obtenues.iter().map(|l| l.sort_order).collect::<Vec<_>>(),
            (0..obtenues.len() as i64).collect::<Vec<_>>(),
            "{etiquette} : renumérotation"
        );
        // Et le Python avait bien produit la même numérotation.
        assert_eq!(
            f.lignes.iter().map(|l| l.sort_order).collect::<Vec<_>>(),
            (0..f.lignes.len() as i64).collect::<Vec<_>>(),
            "{etiquette} : renumérotation du Python"
        );

        vues += f.lignes.len();
    }
    // Les huit sorties dédoublonnées ne sont pas rejouées deux fois : le
    // compte attendu est celui des cas qui portent un fichier.
    let attendues: usize = index
        .cas
        .iter()
        .filter(|c| c.fichier.is_some())
        .map(|c| c.lignes)
        .sum();
    assert_eq!(
        vues, attendues,
        "toutes les lignes figées doivent être rejouées"
    );
    assert_eq!(vues, 4291);
}

#[test]
fn l_overframe_forme_un_bloc_place_apres() {
    // LOCR-JP de la V1.0.4 est le seul classeur capturé qui en porte.
    let f: Fixture = lire(dossier().join("V1.0.4_LOCR-JP.json"));
    let ext: Vec<_> = f.lignes.iter().filter(|l| l.extended_art != 0).collect();
    assert_eq!(ext.len(), 54);

    // Pour chaque numéro qui a les deux cadres, toutes les lignes normales
    // précèdent toutes les Overframe.
    for ligne in &f.lignes {
        if ligne.extended_art == 0 {
            continue;
        }
        let dernier_normal = f
            .lignes
            .iter()
            .filter(|l| l.extended_art == 0 && cle_tri(&l.set_code) == cle_tri(&ligne.set_code))
            .map(|l| l.sort_order)
            .max();
        if let Some(rang) = dernier_normal {
            assert!(
                ligne.sort_order > rang,
                "{} : l'Overframe doit suivre le cadre normal",
                ligne.set_code
            );
        }
    }
}

#[test]
fn l_heuristique_d_incompletude_suit_le_python() {
    let dossier = dossier();
    let index: Index = lire(dossier.join("index.json"));

    for cas in &index.cas {
        let Some(fichier) = &cas.fichier else {
            continue;
        };
        let f: Fixture = lire(dossier.join(fichier));
        let etiquette = format!("{}/{}", cas.version, f.classeur);

        assert_eq!(
            f.incompletude.seuil, SEUIL_AVG_RARETES_SUSPECT,
            "{etiquette} : le seuil du portage doit être celui du Python"
        );

        let obtenu = locales_semblent_incompletes(&en_lignes(&f.lignes));
        assert_eq!(
            obtenu.suspect, f.incompletude.suspect,
            "{etiquette} : verdict (moyenne {:.4})",
            f.incompletude.moyenne
        );
        assert_eq!(
            obtenu.cartes_uniques, f.incompletude.cartes_uniques,
            "{etiquette} : cartes distinctes"
        );
        assert!(
            (obtenu.moyenne - f.incompletude.moyenne).abs() < 1e-9,
            "{etiquette} : moyenne {} contre {}",
            obtenu.moyenne,
            f.incompletude.moyenne
        );
        // L'index doit dire la même chose que la fixture.
        assert_eq!(cas.suspect, f.incompletude.suspect);
        assert_eq!(cas.cartes_uniques, f.incompletude.cartes_uniques);
        assert!((cas.moyenne - f.incompletude.moyenne).abs() < 1e-9);
    }
}

#[test]
fn les_sorties_annoncees_identiques_le_sont_vraiment() {
    // Huit classeurs donnent la même sortie sur les deux bases : leurs données
    // n'ont pas bougé entre les deux constructions. La fixture ne les stocke
    // qu'une fois — encore faut-il que la référence existe.
    let dossier = dossier();
    let index: Index = lire(dossier.join("index.json"));
    let dedup: Vec<_> = index
        .cas
        .iter()
        .filter(|c| c.identique_a.is_some())
        .collect();
    assert_eq!(dedup.len(), 8);

    for cas in dedup {
        let cible = cas.identique_a.as_deref().unwrap();
        assert!(
            dossier.join(cible).is_file(),
            "{}/{} renvoie vers {cible}, absent",
            cas.version,
            cas.classeur
        );
        let f: Fixture = lire(dossier.join(cible));
        assert_eq!(f.lignes.len(), cas.lignes);
        assert_eq!(f.classeur, cas.classeur);
    }
}

#[test]
fn le_numero_ordinal_est_celui_de_la_fin_du_code() {
    // `ordre_depuis_code` ne sert que de valeur initiale, mais il diffère de la
    // clé de tri et le test le montre : il se contente des chiffres de fin.
    for cas in ["SS01-ENA01", "RA05-EN035", "LOB-EN001"] {
        assert_eq!(ordre_depuis_code(cas), ordre_depuis_code(cas));
    }
    assert_eq!(ordre_depuis_code("SS01-ENA01"), 1);
    assert_eq!(ordre_depuis_code("RA05-EN035"), 35);
    assert_eq!(ordre_depuis_code("LOB-EN001"), 1);
    assert_eq!(ordre_depuis_code(""), 0);
    // Là où la clé de tri renonce, le numéro ordinal trouve quand même.
    assert_eq!(cle_tri("SET-EN00X"), ("", 0));
    assert_eq!(
        ordre_depuis_code("SET-EN12X"),
        0,
        "les chiffres doivent finir le code"
    );
    assert_eq!(ordre_depuis_code("SET-ENX12"), 12);
}
