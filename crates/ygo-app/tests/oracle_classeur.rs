// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les données de l'écran classeur, sur les 26 classeurs réels.
//!
//! # La moitié prouvable du plus gros écran
//!
//! `ecran_classeur.py` fait 1 726 lignes, dont l'essentiel est du dessin. Ce
//! qu'il **calcule** — la liste des cartes résolue et triée, les raretés
//! proposées, la chaîne de filtres, et la pagination en doubles pages — se
//! compare champ par champ à ce que le Python produit.
//!
//! # Les réglages sont figés avec les données
//!
//! `get_cartes_info` dépend des préférences de l'utilisateur : langue, source
//! d'images, ordre de tri, priorités de rareté. Sans les figer, l'oracle
//! dériverait **en silence** le jour où l'une d'elles change — le portage
//! serait comparé à un gel obtenu sous d'autres règles. Chaque fixture porte
//! donc ses `reglages`, et ces tests les **exigent**.
//!
//! # Ce que cet oracle ne prouve PAS
//!
//! Les 26 classeurs sont **tous en 3×3**. La grille par classeur — 4×3, 4×4,
//! ce que l'utilisateur veut — n'est donc jamais exercée ici, alors qu'elle
//! commande toute la pagination. Les tests unitaires du module s'en chargent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use ygo_app::classeur::{self, Filtres, Grille, Possession};
use ygo_core::config::{CritereTri, SourceImage};
use ygo_core::rarity::Priorites;

#[derive(Deserialize)]
struct Fixture {
    version: String,
    classeur: String,
    colonnes: u8,
    lignes: u8,
    cartes: Vec<CarteOracle>,
    raretes: Vec<String>,
    nb_doubles_pages: usize,
    doubles_pages: Vec<DoublePageOracle>,
    filtres: Vec<FiltreOracle>,
    reglages: Reglages,
}

#[derive(Deserialize)]
struct Reglages {
    colonne_nom: String,
    source_image: String,
    ordre_tri: Vec<String>,
    priorites_raretes: BTreeMap<String, i64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
struct CarteOracle {
    rowid: i64,
    nom: String,
    rarete: String,
    set_code: String,
    card_image_id: i64,
    fichier_image: Option<String>,
    quantite: i64,
    possedee: bool,
    sort_order: i64,
    is_custom: bool,
    extended_art: i64,
}

#[derive(Deserialize)]
struct DoublePageOracle {
    indice: usize,
    gauche: Vec<i64>,
    droite: Vec<i64>,
}

#[derive(Deserialize)]
struct FiltreOracle {
    terme: String,
    rarete: String,
    possession: String,
    rowids: Vec<i64>,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/oracle/classeur")
}

fn lire<T: serde::de::DeserializeOwned>(chemin: PathBuf) -> T {
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte)
        .unwrap_or_else(|e| panic!("fixture {} illisible : {e}", chemin.display()))
}

/// Toutes les fixtures, dans un ordre stable.
fn toutes() -> Vec<Fixture> {
    let mut chemins: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .expect("dossier de fixtures")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("V1.") && n.ends_with(".json"))
        })
        .collect();
    chemins.sort();
    assert_eq!(chemins.len(), 26, "26 classeurs figés");
    chemins.into_iter().map(lire).collect()
}

/// Les critères de tri, depuis les libellés figés.
fn ordre(reglages: &Reglages) -> [CritereTri; 3] {
    let lus: Vec<CritereTri> = reglages
        .ordre_tri
        .iter()
        .map(|code| {
            CritereTri::depuis_code(code).unwrap_or_else(|| panic!("critère inconnu : {code}"))
        })
        .collect();
    [lus[0], lus[1], lus[2]]
}

/// Les priorités de rareté, chargées par le **vrai** lecteur.
fn priorites(reglages: &Reglages, dossier: &std::path::Path) -> Priorites {
    let chemin = dossier.join("rarity_config.json");
    std::fs::write(
        &chemin,
        serde_json::to_string(&reglages.priorites_raretes).unwrap(),
    )
    .unwrap();
    Priorites::charger(&chemin)
}

/// Rebâtit un classeur jouable depuis les cartes figées.
///
/// # Pourquoi une reconstruction plutôt que la vraie base
///
/// Les installations sont sur le poste de l'utilisateur. Un test qui se saute
/// est un test qui passe à vide : celui-ci reconstruit, sur le **vrai DDL de
/// classeur**, une base qui porte les mêmes cartes, et tourne partout.
///
/// Le nom résolu est écrit dans `name_fr` et un leurre dans `name` : la
/// requête doit donc rendre le premier, ce qui éprouve au passage le
/// `COALESCE(NULLIF(name_fr,''), name)`.
///
/// L'URL est reconstituée pour que le nom de fichier attendu retombe : les
/// identifiants positifs passent par l'URL YGOPRODeck construite, les négatifs
/// — artworks externes — par l'URL stockée.
fn classeur_depuis(cartes: &[CarteOracle], dossier: &std::path::Path) -> rusqlite::Connection {
    let conn = ygo_db::connexion::ouvrir(dossier.join("classeur.db")).expect("base");
    ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).expect("schéma");
    for c in cartes {
        let url = c.fichier_image.as_ref().map(|f| {
            if c.card_image_id > 0 {
                format!("https://images.ygoprodeck.com/images/cards/{f}")
            } else {
                format!("https://ms.yugipedia.com/a/ab/{f}")
            }
        });
        conn.execute(
            "INSERT INTO cards (rowid, name, name_fr, rarity, set_code, set_name, \
                 possessed, quantite, sort_order, card_image_id, card_image_url, \
                 is_custom, extended_art) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'Le Set', ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                c.rowid,
                format!("LEURRE {}", c.rowid),
                c.nom,
                c.rarete,
                c.set_code,
                i64::from(c.possedee),
                c.quantite,
                c.sort_order,
                (c.card_image_id != 0).then_some(c.card_image_id),
                url,
                i64::from(c.is_custom),
                c.extended_art,
            ],
        )
        .expect("insertion");
    }
    conn
}

#[test]
fn les_reglages_sont_les_memes_pour_les_26_classeurs() {
    // Si un jour ils divergent, chaque fixture devra être jouée sous les
    // siens — mieux vaut le découvrir par un test que par un écart inexpliqué.
    for f in toutes() {
        assert_eq!(f.reglages.colonne_nom, "name_fr", "{}", f.classeur);
        assert_eq!(f.reglages.source_image, "YGOPRODECK", "{}", f.classeur);
        assert_eq!(
            f.reglages.ordre_tri,
            ["numero", "artwork", "rarete"],
            "{}",
            f.classeur
        );
    }
}

#[test]
fn les_cartes_sont_celles_du_python_dans_le_meme_ordre() {
    let temporaire = tempfile::tempdir().unwrap();
    let mut total = 0usize;

    for f in toutes() {
        let ou = format!("{} / {}", f.version, f.classeur);
        let dossier = temporaire
            .path()
            .join(format!("{}_{}", f.version, f.classeur));
        std::fs::create_dir_all(&dossier).unwrap();

        let conn = classeur_depuis(&f.cartes, &dossier);
        let priorites = priorites(&f.reglages, &dossier);
        let obtenues = classeur::charger(
            &conn,
            true,
            SourceImage::Ygoprodeck,
            ordre(&f.reglages),
            &priorites,
        )
        .expect("chargement");

        assert_eq!(obtenues.len(), f.cartes.len(), "{ou} : nombre de cartes");
        for (obtenue, attendue) in obtenues.iter().zip(&f.cartes) {
            assert_eq!(obtenue.rowid, attendue.rowid, "{ou} : ordre des cartes");
            assert_eq!(obtenue.nom, attendue.nom, "{ou} #{}: nom", attendue.rowid);
            assert_eq!(obtenue.rarete, attendue.rarete, "{ou} #{}", attendue.rowid);
            assert_eq!(
                obtenue.set_code, attendue.set_code,
                "{ou} #{}",
                attendue.rowid
            );
            assert_eq!(
                obtenue.card_image_id, attendue.card_image_id,
                "{ou} #{}",
                attendue.rowid
            );
            assert_eq!(
                obtenue.fichier_image, attendue.fichier_image,
                "{ou} #{} : nom de fichier d'image",
                attendue.rowid
            );
            assert_eq!(
                obtenue.quantite, attendue.quantite,
                "{ou} #{}",
                attendue.rowid
            );
            assert_eq!(
                obtenue.possedee, attendue.possedee,
                "{ou} #{}",
                attendue.rowid
            );
            assert_eq!(
                obtenue.extended_art,
                attendue.extended_art != 0,
                "{ou} #{}",
                attendue.rowid
            );
        }

        assert_eq!(
            classeur::raretes_disponibles(&obtenues),
            f.raretes,
            "{ou} : raretés proposées par le filtre"
        );
        total += obtenues.len();
    }
    assert!(total > 6000, "obtenu {total} cartes comparées");
}

#[test]
fn la_pagination_en_doubles_pages_est_celle_du_python() {
    let temporaire = tempfile::tempdir().unwrap();

    for f in toutes() {
        let ou = format!("{} / {}", f.version, f.classeur);
        let dossier = temporaire
            .path()
            .join(format!("{}_{}", f.version, f.classeur));
        std::fs::create_dir_all(&dossier).unwrap();
        let conn = classeur_depuis(&f.cartes, &dossier);
        let priorites = priorites(&f.reglages, &dossier);
        let cartes = classeur::charger(
            &conn,
            true,
            SourceImage::Ygoprodeck,
            ordre(&f.reglages),
            &priorites,
        )
        .expect("chargement");

        let grille = Grille {
            colonnes: f.colonnes,
            lignes: f.lignes,
        };
        assert_eq!(
            classeur::nb_doubles_pages(cartes.len(), grille),
            f.nb_doubles_pages,
            "{ou} : nombre de doubles pages"
        );

        for attendue in &f.doubles_pages {
            let (gauche, droite) = classeur::double_page(&cartes, attendue.indice, grille);
            let ids = |c: &[ygo_app::classeur::Carte]| -> Vec<i64> {
                c.iter().map(|x| x.rowid).collect()
            };
            assert_eq!(
                ids(gauche),
                attendue.gauche,
                "{ou} : page gauche de la double page {}",
                attendue.indice
            );
            assert_eq!(
                ids(droite),
                attendue.droite,
                "{ou} : page droite de la double page {}",
                attendue.indice
            );
        }

        // La première double page ne montre qu'une page, à droite.
        assert!(
            classeur::double_page(&cartes, 0, grille).0.is_empty(),
            "{ou} : la première page est seule à droite"
        );
    }
}

#[test]
fn les_filtres_rendent_les_memes_cartes_que_le_python() {
    let temporaire = tempfile::tempdir().unwrap();

    for f in toutes() {
        let ou = format!("{} / {}", f.version, f.classeur);
        let dossier = temporaire
            .path()
            .join(format!("{}_{}", f.version, f.classeur));
        std::fs::create_dir_all(&dossier).unwrap();
        let conn = classeur_depuis(&f.cartes, &dossier);
        let priorites = priorites(&f.reglages, &dossier);
        let cartes = classeur::charger(
            &conn,
            true,
            SourceImage::Ygoprodeck,
            ordre(&f.reglages),
            &priorites,
        )
        .expect("chargement");

        for essai in &f.filtres {
            let filtres = Filtres {
                terme: essai.terme.clone(),
                rarete: (essai.rarete != "Toutes").then(|| essai.rarete.clone()),
                possession: match essai.possession.as_str() {
                    "Possedees" => Possession::Possedees,
                    "NonPossedees" => Possession::NonPossedees,
                    _ => Possession::Toutes,
                },
                // L'oracle fige les filtres AVANT « N raretés par artwork » :
                // ce dernier a son propre oracle dans `ygo-core::tri`.
                n_raretes: 0,
            };
            let obtenues = classeur::appliquer(cartes.clone(), &filtres, &priorites);
            let ids: Vec<i64> = obtenues.iter().map(|c| c.rowid).collect();
            assert_eq!(
                ids, essai.rowids,
                "{ou} : filtre terme={:?} rareté={:?} possession={:?}",
                essai.terme, essai.rarete, essai.possession
            );
        }
    }
}

#[test]
fn les_26_classeurs_sont_tous_en_trois_par_trois() {
    // Ce que l'oracle NE couvre pas, dit par un test plutôt que laissé à
    // découvrir : la grille par classeur commande toute la pagination, et
    // aucune donnée réelle ne s'écarte du 3×3 par défaut.
    for f in toutes() {
        assert_eq!(
            (f.colonnes, f.lignes),
            (3, 3),
            "{} / {} — si ce test tombe, l'oracle vient de gagner en portée",
            f.version,
            f.classeur
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// La canonisation des raretés, mesurée sur les mêmes 26 classeurs
//
// Ces tests ne comparent rien au Python : la canonisation est un écart
// délibéré, demandé par l'utilisateur. Ils mesurent son effet sur les données
// réelles, et surtout ils **gardent les chiffres annoncés** — un changement de
// table ou de règle qui déplacerait la couverture les rendrait rouges.
// ─────────────────────────────────────────────────────────────────────────────

/// La liste des Options réelle de l'utilisateur.
fn options_reelles() -> Priorites {
    let texte = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/rarity_config.reference.json"),
    )
    .expect("rarity_config de référence");
    let tmp = tempfile::tempdir().expect("dossier temporaire");
    let chemin = tmp.path().join("rarity_config.json");
    std::fs::write(&chemin, texte).expect("écriture");
    Priorites::charger(&chemin)
}

#[test]
fn l_ampleur_du_probleme_est_bien_celle_annoncee() {
    let options = options_reelles();
    let mut lignes = 0_usize;
    let mut non_canoniques: BTreeMap<String, usize> = BTreeMap::new();

    for fixture in toutes() {
        for carte in &fixture.cartes {
            lignes += 1;
            if options.brute(&carte.rarete).is_none() {
                *non_canoniques.entry(carte.rarete.clone()).or_insert(0) += 1;
            }
        }
    }

    assert_eq!(lignes, 7_136, "7 136 lignes sur les 26 classeurs");
    let total: usize = non_canoniques.values().sum();
    assert_eq!(total, 912, "912 lignes hors de la liste des Options");
    assert_eq!(non_canoniques.len(), 10, "10 écritures distinctes");
    assert_eq!(non_canoniques.get("PLatinum Secret Rare"), Some(&160));
    assert_eq!(non_canoniques.get("UR"), Some(&103));
    assert_eq!(non_canoniques.get("force-SMW"), Some(&1));
}

/// Ce que la table actuelle résout, et ce qu'il reste à capturer.
///
/// La seule ligne qui résiste est `force-SMW`, qui n'est pas une rareté et ne
/// doit jamais en devenir une. Les 911 autres sont ramenées à la forme de la
/// liste des Options.
#[test]
fn la_canonisation_couvre_911_des_912_lignes() {
    let options = options_reelles();
    let mut resolues = 0_usize;
    let mut irreductibles: BTreeMap<String, usize> = BTreeMap::new();

    for fixture in toutes() {
        for carte in &fixture.cartes {
            if options.brute(&carte.rarete).is_some() {
                continue;
            }
            match ygo_core::rarity::canon::canoniser(&carte.rarete, &options) {
                Some(_) => resolues += 1,
                None => *irreductibles.entry(carte.rarete.clone()).or_insert(0) += 1,
            }
        }
    }

    assert_eq!(resolues, 911, "911 des 912 lignes canonisées");
    assert_eq!(
        irreductibles.keys().collect::<Vec<_>>(),
        ["force-SMW"],
        "seule l'annotation résiste, et c'est voulu"
    );
    assert_eq!(irreductibles.get("force-SMW"), Some(&1));
}

/// Le fond du sujet : ces lignes sont **déjà mal triées**. Une rareté que la
/// liste des Options ne connaît pas part en `9999` au tri et en `0` au filtre.
/// La canonisation les ramène à leur vraie place.
#[test]
fn les_lignes_non_canoniques_sont_mal_triees_avant_correction() {
    let options = options_reelles();

    assert_eq!(
        options.pour_tri("UR"),
        ygo_core::rarity::PRIORITE_INCONNUE_TRI
    );
    assert_eq!(
        options.pour_filtre("UR"),
        ygo_core::rarity::PRIORITE_INCONNUE_FILTRE
    );

    let canon = ygo_core::rarity::canon::canoniser("UR", &options).expect("résolue");
    assert_eq!(
        options.pour_tri(&canon.libelle),
        4,
        "Ultra Rare, priorité 4"
    );
    assert_eq!(options.pour_filtre(&canon.libelle), 4);
}

/// `RA02` est écrit deux fois : 553 lignes en libellés complets, qui portent
/// les 21 cartes possédées, et 567 lignes en abréviations, toutes à zéro. La
/// canonisation les rendra identiques ; le dédoublonnage est une opération
/// distincte, et ce test fige l'état de départ.
#[test]
fn ra02_porte_le_meme_set_sous_deux_vocabulaires() {
    let fixture = toutes()
        .into_iter()
        .find(|f| f.classeur == "RA02" && f.version == "V1.0.4")
        .expect("RA02 V1.0.4");

    // Le critère est « la liste des Options connaît-elle ce libellé ? », et
    // surtout pas la longueur : `Rare` fait quatre caractères et est
    // parfaitement canonique — `VASM` n'a que des libellés de ce genre, et un
    // classement par longueur l'aurait déclaré entièrement abrégé.
    let options = options_reelles();
    let abregees: Vec<&CarteOracle> = fixture
        .cartes
        .iter()
        .filter(|c| options.brute(&c.rarete).is_none())
        .collect();
    let completes: Vec<&CarteOracle> = fixture
        .cartes
        .iter()
        .filter(|c| options.brute(&c.rarete).is_some())
        .collect();

    assert_eq!(completes.len(), 553);
    assert_eq!(abregees.len(), 567);
    assert_eq!(
        completes.iter().filter(|c| c.quantite > 0).count(),
        21,
        "les cartes possédées sont du côté des libellés complets"
    );
    assert_eq!(
        abregees.iter().filter(|c| c.quantite > 0).count(),
        0,
        "aucune quantité du côté abrégé — le doublon est vide"
    );
}

/// Le dédoublonnage, mesuré sur les 26 classeurs figés.
///
/// Les chiffres viennent des bases réelles ; ce test les garde. Il exerce la
/// **règle** de [`ygo_app::doublons::planifier`] sur les fixtures, pas une
/// réimplémentation : c'est la même fonction que la commande appelle.
#[test]
fn le_dedoublonnage_rend_ra02_a_sa_taille_de_creation() {
    let options = options_reelles();
    let fixture = toutes()
        .into_iter()
        .find(|f| f.classeur == "RA02" && f.version == "V1.0.4")
        .expect("RA02 V1.0.4");

    let lignes: Vec<ygo_app::doublons::Ligne> = fixture
        .cartes
        .iter()
        .map(|c| ygo_app::doublons::Ligne {
            rowid: c.rowid,
            set_code: c.set_code.clone(),
            rarity: c.rarete.clone(),
            extended_art: c.extended_art,
            quantite: c.quantite,
        })
        .collect();

    let rapport = ygo_app::doublons::planifier(&lignes, &options);

    assert_eq!(rapport.lues, 1_120);
    assert_eq!(
        rapport.suppressions.len(),
        567,
        "les lignes insérées à tort"
    );
    assert_eq!(
        rapport.lues - rapport.suppressions.len(),
        553,
        "exactement ce que `ygo-cli creer RA02` écrit"
    );
    assert!(
        rapport.bloquees.is_empty(),
        "aucune ligne à supprimer ne porte de quantité"
    );
    assert!(rapport.sans_temoin.is_empty());
}

/// Le contre-exemple, et il est essentiel : `RA05` porte 80 lignes
/// `PLatinum Secret Rare` qui **ne doublent personne**. Une règle qui les
/// emporterait effacerait 80 cartes.
#[test]
fn le_dedoublonnage_ne_touche_pas_ra05() {
    let options = options_reelles();
    for fixture in toutes()
        .into_iter()
        .filter(|f| ["RA05", "LOCR-JP", "SDWD", "VASM"].contains(&f.classeur.as_str()))
    {
        let lignes: Vec<ygo_app::doublons::Ligne> = fixture
            .cartes
            .iter()
            .map(|c| ygo_app::doublons::Ligne {
                rowid: c.rowid,
                set_code: c.set_code.clone(),
                rarity: c.rarete.clone(),
                extended_art: c.extended_art,
                quantite: c.quantite,
            })
            .collect();

        let rapport = ygo_app::doublons::planifier(&lignes, &options);
        assert!(
            rapport.vide(),
            "{} {} : {} suppression(s) alors qu'il n'y a aucun doublon",
            fixture.version,
            fixture.classeur,
            rapport.suppressions.len()
        );
        assert!(rapport.bloquees.is_empty());
    }
}

/// Le total, sur les neuf classeurs de la V1.0.4.
#[test]
fn le_dedoublonnage_retire_751_lignes_sur_la_v1_0_4() {
    let options = options_reelles();
    let mut supprimees = 0_usize;
    let mut bloquees = 0_usize;

    for fixture in toutes().into_iter().filter(|f| f.version == "V1.0.4") {
        let lignes: Vec<ygo_app::doublons::Ligne> = fixture
            .cartes
            .iter()
            .map(|c| ygo_app::doublons::Ligne {
                rowid: c.rowid,
                set_code: c.set_code.clone(),
                rarity: c.rarete.clone(),
                extended_art: c.extended_art,
                quantite: c.quantite,
            })
            .collect();
        let rapport = ygo_app::doublons::planifier(&lignes, &options);
        supprimees += rapport.suppressions.len();
        bloquees += rapport.bloquees.len();
    }

    assert_eq!(supprimees, 751);
    assert_eq!(bloquees, 0, "aucune ligne possédée n'est en jeu");
}
