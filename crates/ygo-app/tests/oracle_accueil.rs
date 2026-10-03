// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les données de l'accueil, sur les 26 classeurs réels des deux installations.
//!
//! # La moitié prouvable du premier lot d'interface
//!
//! On ne fige pas des pixels : l'écran lui-même n'aura pas d'oracle. Mais tout
//! ce qu'il affiche est calculé hors de lui, et cette part-là se compare champ
//! par champ à ce que le Python produit — nom du set, compte de cartes,
//! possédées, grille, chemin de couverture et **la piste qui l'a fournie**.
//!
//! `outils/oracle_accueil.py` rejoue la boucle de `_load_data` en appelant les
//! vrais services (`get_set_title`, `get_classeur_meta_full`,
//! `get_stats_collection`, `find_local_booster`) sans démarrer Tk.
//!
//! # Ce que cet oracle ne prouve PAS
//!
//! **Les 26 classeurs ont tous une cover de booster.** Les trois autres pistes
//! de `chercher_couverture` — la carte du classeur, l'ancien dossier par set,
//! et l'absence — ne sont donc **jamais** exercées ici. Elles le sont par les
//! tests unitaires, sur une arborescence bâtie pour ça. C'est la même leçon
//! que le lot artworks : un oracle sur données réelles prouve ce que les
//! données contiennent, pas ce que le code peut rencontrer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

use ygo_app::accueil;
use ygo_core::paths::Paths;

#[derive(Deserialize)]
struct Fixture {
    version: String,
    total_cartes: usize,
    total_possedees: usize,
    classeurs: Vec<ClasseurOracle>,
}

#[derive(Deserialize, Debug)]
struct ClasseurOracle {
    code: String,
    nom: String,
    nom_fr: String,
    total: usize,
    possedees: usize,
    colonnes: u8,
    lignes: u8,
    image_id: Option<serde_json::Value>,
    couverture: Option<String>,
    origine_couverture: String,
    pourcentage: f64,
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

/// La racine d'une installation Python réelle, si elle est sur ce disque.
///
/// Les installations sont sur le poste de l'utilisateur, pas dans le dépôt :
/// ces deux tests-là se **sautent** ailleurs. Un test qui se saute est un test
/// qui passe à vide — c'est pourquoi ils ne sont pas seuls : les deux tests
/// « installation miniature » ci-dessus rejouent la même comparaison sur une
/// arborescence rebâtie, et tournent partout. Ceux-ci ajoutent ce qu'aucune
/// reconstruction ne donne : le vrai disque de l'utilisateur.
fn installation(version: &str) -> Option<PathBuf> {
    let racine = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../Projet Python")
        .join(version);
    racine.join("bdd/cardinfo.db").is_file().then_some(racine)
}

fn comparer(version: &str) {
    let Some(racine) = installation(version) else {
        eprintln!("{version} absente de ce disque — test sauté");
        return;
    };
    let f: Fixture = lire(fixtures().join(format!("oracle/accueil/{version}.json")));
    assert_eq!(f.version, version);

    let paths = Paths::depuis_racine(&racine);
    let obtenus = accueil::lister(&paths, (3, 3));

    assert_eq!(
        obtenus.len(),
        f.classeurs.len(),
        "{version} : nombre de classeurs"
    );

    for (obtenu, attendu) in obtenus.iter().zip(&f.classeurs) {
        let ou = format!("{version} / {}", attendu.code);
        assert_eq!(obtenu.code, attendu.code, "{ou} : code");
        assert_eq!(obtenu.nom, attendu.nom, "{ou} : nom");
        assert_eq!(obtenu.nom_fr, attendu.nom_fr, "{ou} : nom français");
        assert_eq!(obtenu.total, attendu.total, "{ou} : total");
        assert_eq!(obtenu.possedees, attendu.possedees, "{ou} : possédées");
        assert_eq!(obtenu.colonnes, attendu.colonnes, "{ou} : colonnes");
        assert_eq!(obtenu.lignes, attendu.lignes, "{ou} : lignes");
        assert_eq!(
            obtenu.origine_couverture.code(),
            attendu.origine_couverture,
            "{ou} : d'où vient la couverture"
        );

        // Le chemin est figé en relatif ; on le recompose pour comparer.
        let attendue = attendu
            .couverture
            .as_ref()
            .map(|c| racine.join(Path::new(&c.replace('/', std::path::MAIN_SEPARATOR_STR))));
        assert_eq!(obtenu.couverture, attendue, "{ou} : chemin de couverture");

        let id_attendu = attendu.image_id.as_ref().map(|v| match v {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            autre => panic!("{ou} : image_id inattendu {autre:?}"),
        });
        assert_eq!(obtenu.image_id, id_attendu, "{ou} : image_id");

        assert!(
            (obtenu.pourcentage() - attendu.pourcentage).abs() < 1e-9,
            "{ou} : pourcentage — {} contre {}",
            obtenu.pourcentage(),
            attendu.pourcentage
        );
    }

    let (total, poss) = accueil::totaux(&obtenus);
    assert_eq!(total, f.total_cartes, "{version} : total de cartes");
    assert_eq!(poss, f.total_possedees, "{version} : total possédées");
}

/// Rebâtit une installation miniature depuis la fixture.
///
/// # Pourquoi ce détour
///
/// Les deux installations Python sont sur le poste de l'utilisateur. Les tests
/// qui les visent se sautent ailleurs — et un test qui se saute est un test qui
/// **passe à vide**. Celui-ci reconstruit, depuis la fixture, une arborescence
/// qui a les mêmes comptes, les mêmes grilles, les mêmes noms et les mêmes
/// covers, sur le **vrai DDL de classeur**. Il tourne partout, et il exerce
/// pour de bon les requêtes SQL et la recherche de couverture.
fn installation_miniature(f: &Fixture, base: &Path) -> PathBuf {
    let racine = base.join(&f.version);
    let classeurs = racine.join("bdd/classeur_creer");
    let boosters = racine.join("img/boosters");
    std::fs::create_dir_all(&classeurs).unwrap();
    std::fs::create_dir_all(&boosters).unwrap();
    std::fs::create_dir_all(racine.join("img/small")).unwrap();

    for c in &f.classeurs {
        let dossier = classeurs.join(&c.code);
        std::fs::create_dir_all(&dossier).unwrap();
        let conn = ygo_db::connexion::ouvrir(dossier.join(format!("{}.db", c.code))).unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();

        let id = c.image_id.as_ref().map(|v| match v {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            autre => panic!("image_id inattendu {autre:?}"),
        });
        for i in 0..c.total {
            conn.execute(
                "INSERT INTO cards (name, set_code, rarity, set_name, possessed, card_image_id) \
                 VALUES (?1, ?2, 'Ultra Rare', ?3, ?4, ?5)",
                rusqlite::params![
                    format!("Carte {i}"),
                    format!("{}-{i:03}", c.code),
                    c.nom,
                    i64::from(i < c.possedees),
                    // Seule la première ligne porte l'identifiant : c'est elle
                    // que `LIMIT 1` doit trouver.
                    if i == 0 { id.clone() } else { None },
                ],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('colonnes', ?1), ('lignes', ?2)",
            [c.colonnes.to_string(), c.lignes.to_string()],
        )
        .unwrap();

        if let Some(chemin) = &c.couverture {
            let nom = Path::new(chemin).file_name().unwrap();
            std::fs::write(boosters.join(nom), b"PNG factice").unwrap();
        }
    }
    racine
}

fn comparer_miniature(version: &str) {
    let f: Fixture = lire(fixtures().join(format!("oracle/accueil/{version}.json")));
    let temporaire = tempfile::tempdir().unwrap();
    let racine = installation_miniature(&f, temporaire.path());

    let paths = Paths::depuis_racine(&racine);
    let obtenus = accueil::lister(&paths, (3, 3));
    assert_eq!(obtenus.len(), f.classeurs.len());

    for (obtenu, attendu) in obtenus.iter().zip(&f.classeurs) {
        let ou = format!("{version} / {}", attendu.code);
        assert_eq!(obtenu.code, attendu.code, "{ou} : code");
        assert_eq!(obtenu.nom, attendu.nom, "{ou} : nom");
        assert_eq!(obtenu.total, attendu.total, "{ou} : total");
        assert_eq!(obtenu.possedees, attendu.possedees, "{ou} : possédées");
        assert_eq!(obtenu.colonnes, attendu.colonnes, "{ou} : colonnes");
        assert_eq!(obtenu.lignes, attendu.lignes, "{ou} : lignes");
        assert_eq!(
            obtenu.origine_couverture.code(),
            attendu.origine_couverture,
            "{ou} : d'où vient la couverture"
        );
        assert!(
            (obtenu.pourcentage() - attendu.pourcentage).abs() < 1e-9,
            "{ou} : pourcentage"
        );
    }

    let (total, poss) = accueil::totaux(&obtenus);
    assert_eq!(total, f.total_cartes, "{version} : total de cartes");
    assert_eq!(poss, f.total_possedees, "{version} : total possédées");
}

#[test]
fn les_17_classeurs_de_la_v103_rejoues_sur_une_installation_miniature() {
    comparer_miniature("V1.0.3");
}

#[test]
fn les_9_classeurs_de_la_v104_rejoues_sur_une_installation_miniature() {
    comparer_miniature("V1.0.4");
}

#[test]
fn l_accueil_reproduit_les_17_classeurs_de_la_v103() {
    comparer("V1.0.3");
}

#[test]
fn l_accueil_reproduit_les_9_classeurs_de_la_v104() {
    comparer("V1.0.4");
}

/// Ce que les deux installations ont en commun, et qui mérite d'être dit.
#[test]
fn les_deux_installations_n_exercent_qu_une_seule_piste_de_couverture() {
    let v103: Fixture = lire(fixtures().join("oracle/accueil/V1.0.3.json"));
    let v104: Fixture = lire(fixtures().join("oracle/accueil/V1.0.4.json"));
    assert_eq!(v103.classeurs.len(), 17);
    assert_eq!(v104.classeurs.len(), 9);

    let pistes: std::collections::BTreeSet<&str> = v103
        .classeurs
        .iter()
        .chain(&v104.classeurs)
        .map(|c| c.origine_couverture.as_str())
        .collect();
    assert_eq!(
        pistes,
        ["booster"].into_iter().collect(),
        "si une autre piste apparaît un jour, ce test le dira — et l'oracle \
         deviendra alors plus contraignant qu'il ne l'est aujourd'hui"
    );
}
