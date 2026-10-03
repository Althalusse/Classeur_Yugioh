// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La passe artworks, rejouée sur le classeur `LOCR-JP` réel.
//!
//! # Un oracle bâti sans une seule requête
//!
//! Les trois entrées de la passe étaient déjà sur le disque :
//!
//! | entrée | d'où |
//! |---|---|
//! | la Set list | `tests/fixtures/net/yugipedia/`, capturée à la révision 5945579 |
//! | le classeur | celui de la V1.0.3, **318 lignes, aucune image externe** |
//!
//! (Le classeur de la V1.0.3 porte déjà ses 54 tirages Overframe : la passe
//! overframe l'avait précédé. Le diff isole donc bien la seule passe artworks
//! — d'où les `0` en `ajoutes` et en `flags` du bilan.)
//! | les images | `card_images_externes` de la V1.0.4, 302 fichiers `LOCR` |
//!
//! `outils/oracle_artworks.py` fait tourner le vrai
//! `completer_artworks_variantes` sur une **copie** du classeur et fige le
//! diff avant/après. C'est exactement l'effet observable de la passe.
//!
//! # Pourquoi le classeur de la V1.0.3
//!
//! Celui de la V1.0.4 a **déjà subi la passe** : la rejouer n'y change rien.
//! C'est une bonne vérification d'idempotence et un mauvais oracle — aucune
//! branche ne s'exerce. Celui de la V1.0.3 est l'état d'avant.
//!
//! # Ce que le diff contient
//!
//! **30 lignes modifiées sur 318** : leur illustration YGOPRODeck cède la place
//! à celle de Yugipedia — identifiant négatif, `card_image_uuid` en
//! `yugipedia:…`. Le test exige la bonne image sur la bonne ligne, fichier par
//! fichier.
//!
//! # Ce que cet oracle ne prouve PAS
//!
//! Il fallait le mesurer plutôt que le supposer, et la mesure a démenti la
//! première lecture. Sur dix mutations du planificateur, **deux seulement**
//! font tomber ce test. Les autres laissent le compte à 30 :
//!
//! - `LOCR` n'a que **10 fichiers suffixés sur 302** — les deux premiers
//!   niveaux de préférence de `choisir_fichier` n'y jouent presque jamais ;
//! - quand la règle des fichiers nus refuse un candidat direct, le **repli sur
//!   les autres raretés** le rattrape : le compte final est identique, seul le
//!   compteur `reutilisees` bouge (0 contre 20).
//!
//! Cet oracle est donc un bon **ancrage de bout en bout** — la bonne image sur
//! la bonne ligne, sur des données réelles — et un mauvais discriminateur de
//! branches. Celles-ci sont couvertes par les dix-huit tests unitaires du
//! module, bâtis sur les cas que le Python documente : `RA05-EN083`,
//! `LOCR-JP001`, et le danger des deux illustrations identiques. Les dix
//! mutations tombent devant eux.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use ygo_app::artworks::{
    appliquer, lire_inventaire, planifier, Action, Bilan, Cible, FichierCandidat, Illustration,
    LigneInventaire, Reference,
};

/// Le libellé de rareté employé par la Set list, par rareté normalisée.
type Libelles = HashMap<String, String>;
use ygo_sources::yugipedia::artwork::{
    abbr_rarete, cle_comparaison, formater_artwork, indexer_fichiers_set, FichierDistant,
};
use ygo_sources::yugipedia::{parse_set_list, structure};

#[derive(Deserialize)]
struct Fixture {
    classeur: String,
    revid: String,
    bilan: BilanOracle,
    avant: Vec<LigneClasseur>,
    apres: Vec<LigneClasseur>,
    images: Vec<ImageOracle>,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
struct BilanOracle {
    ajoutes: usize,
    illustrations: usize,
    flags: usize,
    reutilisees: usize,
    absents: usize,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
struct LigneClasseur {
    rowid: i64,
    name: String,
    set_code: String,
    rarity: String,
    card_image_id: Option<i64>,
    card_image_uuid: Option<String>,
    card_image_url: Option<String>,
    card_image_small: Option<String>,
    extended_art: i64,
    sort_order: i64,
}

#[derive(Deserialize)]
struct ImageOracle {
    card_name: String,
    fichier: String,
    rarete: String,
    variante: String,
    image_id: i64,
    card_url: String,
}

#[derive(Deserialize)]
struct Capture {
    wikitext: String,
    revid: String,
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

/// Reconstruit la référence avec le constructeur **de production**.
///
/// `slots` puis `index_slots` : le même enchaînement que
/// `charger_structure` + `slots_par_numero_rarete`, moins le réseau.
fn reference_depuis_wikitext(wikitext: &str) -> (Reference, Libelles) {
    let slots = structure::slots(&parse_set_list(wikitext));
    (structure::index_slots(&slots), structure::libelles(&slots))
}

/// Les 302 fichiers de la fixture, tels que le réseau les rendrait.
fn distants(images: &[ImageOracle]) -> Vec<FichierDistant> {
    images
        .iter()
        .map(|img| FichierDistant {
            nom: img.fichier.clone(),
            url: img.card_url.clone(),
        })
        .collect()
}

/// Reconstruit l'index des fichiers avec l'indexeur **de production**.
///
/// C'est `fichiers_pour_set` moins sa requête : le test exerce ainsi le vrai
/// code d'indexation au lieu d'une copie écrite pour lui, et vérifie au
/// passage que le découpage Rust redonne les champs que le Python a figés.
fn fichiers_depuis_images(
    images: &[ImageOracle],
    prefixe: &str,
) -> HashMap<(String, String), Vec<FichierCandidat>> {
    let index = indexer_fichiers_set(&distants(images), prefixe, "JP", &[]);
    let attendus: HashMap<&str, &ImageOracle> =
        images.iter().map(|i| (i.fichier.as_str(), i)).collect();

    let mut sortie: HashMap<(String, String), Vec<FichierCandidat>> = HashMap::new();
    let mut indexes = 0usize;
    for (cle, candidats) in index {
        for c in candidats {
            let attendu = attendus
                .get(c.infos.fichier.as_str())
                .unwrap_or_else(|| panic!("fichier inconnu : {}", c.infos.fichier));
            assert_eq!(c.infos.rarete_abbr, attendu.rarete, "{}", c.infos.fichier);
            assert_eq!(c.infos.variante, attendu.variante, "{}", c.infos.fichier);
            assert_eq!(c.image_id, attendu.image_id, "{}", c.infos.fichier);
            assert_eq!(
                c.cle_carte,
                cle_comparaison(&attendu.card_name),
                "{}",
                c.infos.fichier
            );
            indexes += 1;
            sortie
                .entry(cle.clone())
                .or_default()
                .push(FichierCandidat {
                    fichier: c.infos.fichier.clone(),
                    cle_carte: c.cle_carte.clone(),
                    rarete_abbr: c.infos.rarete_abbr.clone(),
                    variante: c.infos.variante.clone(),
                    card_url: c.card_url.clone(),
                    image_id: c.image_id,
                });
        }
    }
    assert_eq!(indexes, images.len(), "aucun fichier perdu à l'indexation");
    sortie
}

/// La liste des Options réelle : la passe artworks canonise désormais les
/// raretés avant de les comparer, et elle a besoin de cette référence.
///
/// Les deux sets figés — `LOCH-JP` et `LOCR-JP` — écrivent leurs raretés en
/// toutes lettres. La canonisation y est donc l'identité, et le fait que ces
/// tests restent verts prouve qu'elle n'a rien déplacé.
fn options_reelles() -> ygo_core::rarity::Priorites {
    let texte = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/rarity_config.reference.json"),
    )
    .expect("rarity_config de référence");
    let tmp = tempfile::tempdir().expect("dossier temporaire");
    let chemin = tmp.path().join("rarity_config.json");
    std::fs::write(&chemin, texte).expect("écriture");
    ygo_core::rarity::Priorites::charger(&chemin)
}

#[test]
fn la_passe_rust_reproduit_le_diff_du_python() {
    let racine = fixtures();
    let f: Fixture = lire(racine.join("oracle/artworks/LOCR-JP.json"));
    let capture: Capture = lire(racine.join("net/yugipedia/wikitext_LOCR-JP_rev5945579.json"));
    assert_eq!(f.classeur, "LOCR-JP");
    assert_eq!(
        capture.revid, f.revid,
        "la capture doit être celle de l'oracle"
    );
    assert_eq!(f.avant.len(), 318);
    assert_eq!(f.images.len(), 302);

    let (reference, libelles) = reference_depuis_wikitext(&capture.wikitext);
    // Ce que la Set list déclare, avant tout appariement. `LOCR` n'annonce
    // aucune variante d'illustration franche : ses 54 variantes sont toutes
    // des cadres étendus. C'est pourquoi la passe ne pose que des `EA` ici —
    // la branche `AA` du constructeur de référence n'est PAS exercée par cet
    // oracle, elle l'est par les tests unitaires de `structure`.
    let familles = reference
        .values()
        .flatten()
        .fold((0, 0, 0), |(n, aa, ea), v| match v.as_str() {
            "" => (n + 1, aa, ea),
            "AA" => (n, aa + 1, ea),
            _ => (n, aa, ea + 1),
        });
    assert_eq!(
        familles,
        (264, 0, 54),
        "normales, AA, EA déclarées par LOCR"
    );

    let fichiers = fichiers_depuis_images(&f.images, "LOCR");
    let inventaire: Vec<LigneInventaire> = f
        .avant
        .iter()
        .filter(|l| !l.name.is_empty())
        .map(|l| LigneInventaire {
            rowid: l.rowid,
            name: l.name.clone(),
            set_code: l.set_code.to_uppercase(),
            rarity: l.rarity.clone(),
            ext: l.extended_art,
        })
        .collect();

    let (actions, bilan) = planifier(
        &reference,
        &libelles,
        &inventaire,
        &fichiers,
        &|r| abbr_rarete(r),
        &|n| cle_comparaison(n),
        &options_reelles(),
    );

    assert_eq!(
        bilan,
        Bilan {
            ajoutes: f.bilan.ajoutes,
            illustrations: f.bilan.illustrations,
            flags: f.bilan.flags,
            reutilisees: f.bilan.reutilisees,
            absents: f.bilan.absents,
        },
        "le bilan doit être celui du Python"
    );

    // ── Le diff produit doit être exactement celui du Python ────────────────
    let avant: HashMap<i64, &LigneClasseur> = f.avant.iter().map(|l| (l.rowid, l)).collect();
    let attendues: HashMap<i64, &LigneClasseur> = f
        .apres
        .iter()
        .filter(|l| avant.get(&l.rowid).is_some_and(|a| *a != *l))
        .map(|l| (l.rowid, l))
        .collect();
    assert_eq!(attendues.len(), 30, "30 lignes modifiées côté Python");

    let mut posees: HashMap<i64, &FichierCandidat> = HashMap::new();
    for action in &actions {
        match action {
            Action::PoserIllustration {
                cible: Cible::Existante(rowid),
                fichier,
                ..
            } => {
                posees.insert(*rowid, fichier);
            }
            Action::PoserIllustration { .. } => unreachable!("aucune ligne créée ici"),
            Action::Inserer { .. } | Action::Flaguer { .. } => {
                unreachable!("le bilan annonce 0 ajout et 0 drapeau")
            }
        }
    }
    assert_eq!(posees.len(), attendues.len(), "mêmes lignes touchées");

    for (rowid, attendue) in &attendues {
        let Some(fichier) = posees.get(rowid) else {
            panic!(
                "ligne {rowid} ({} / {}) modifiée par le Python, ignorée par le portage",
                attendue.set_code, attendue.rarity
            )
        };
        assert_eq!(
            Some(fichier.image_id),
            attendue.card_image_id,
            "ligne {rowid} : identifiant d'image"
        );
        assert_eq!(
            attendue.card_image_uuid.as_deref(),
            Some(format!("yugipedia:{}", fichier.fichier).as_str()),
            "ligne {rowid} : uuid"
        );
        assert_eq!(
            attendue.card_image_url.as_deref(),
            Some(fichier.card_url.as_str()),
            "ligne {rowid} : URL"
        );
    }
}

/// Reconstruit un classeur jouable depuis les lignes figées.
///
/// Le schéma vient du DDL extrait des bases réelles, jamais d'un `CREATE TABLE`
/// recopié : c'est la même règle que pour la création de classeur.
fn classeur_depuis(lignes: &[LigneClasseur]) -> rusqlite::Connection {
    let conn = ygo_db::connexion::en_memoire().expect("base mémoire");
    ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).expect("schéma classeur");
    for l in lignes {
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity, card_image_id, \
                 card_image_uuid, card_image_url, card_image_small, extended_art, sort_order) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                l.rowid,
                l.name,
                l.set_code,
                l.rarity,
                l.card_image_id,
                l.card_image_uuid,
                l.card_image_url,
                l.card_image_small,
                l.extended_art,
                l.sort_order,
            ],
        )
        .expect("insertion");
    }
    conn
}

/// Relit un classeur dans la forme de la fixture, pour comparaison directe.
fn relire(conn: &rusqlite::Connection) -> Vec<LigneClasseur> {
    let mut requete = conn
        .prepare(
            "SELECT rowid, name, set_code, rarity, card_image_id, card_image_uuid, \
                    card_image_url, card_image_small, extended_art, sort_order \
             FROM cards ORDER BY rowid",
        )
        .expect("requête");
    requete
        .query_map([], |l| {
            Ok(LigneClasseur {
                rowid: l.get(0)?,
                name: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                set_code: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                rarity: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
                card_image_id: l.get(4)?,
                card_image_uuid: l.get(5)?,
                card_image_url: l.get(6)?,
                card_image_small: l.get(7)?,
                extended_art: l.get::<_, Option<i64>>(8)?.unwrap_or(0),
                sort_order: l.get::<_, Option<i64>>(9)?.unwrap_or(0),
            })
        })
        .expect("lecture")
        .collect::<rusqlite::Result<_>>()
        .expect("lignes")
}

/// Les illustrations, produites par `formater_artwork` **de production**.
fn illustrations(images: &[ImageOracle]) -> HashMap<String, Illustration> {
    indexer_fichiers_set(&distants(images), "LOCR", "JP", &[])
        .values()
        .flatten()
        .map(|c| {
            let art = formater_artwork(c, "LOCR");
            (
                c.infos.fichier.clone(),
                Illustration {
                    card_image_uuid: art.card_image_uuid,
                    card_image_id: art.card_image_id,
                    card_image_url: art.card_image_url,
                    card_image_small: art.card_image_small,
                },
            )
        })
        .collect()
}

/// La passe entière — lecture, plan, écriture — contre les 318 lignes d'après.
///
/// C'est l'oracle que le test précédent ne pouvait pas être : il compare le
/// **classeur** et non les actions prévues, colonne par colonne, y compris les
/// 288 lignes que la passe ne doit pas toucher.
#[test]
fn la_passe_ecrite_donne_le_classeur_du_python() {
    let racine = fixtures();
    let f: Fixture = lire(racine.join("oracle/artworks/LOCR-JP.json"));
    let capture: Capture = lire(racine.join("net/yugipedia/wikitext_LOCR-JP_rev5945579.json"));

    let conn = classeur_depuis(&f.avant);
    let (inventaire, set_name) = lire_inventaire(&conn).expect("inventaire");
    assert_eq!(inventaire.len(), 318, "toutes les lignes ont un nom");
    assert_eq!(set_name, "", "la fixture ne porte pas set_name");

    let (reference, libelles) = reference_depuis_wikitext(&capture.wikitext);
    let fichiers = fichiers_depuis_images(&f.images, "LOCR");
    let (actions, prevu) = planifier(
        &reference,
        &libelles,
        &inventaire,
        &fichiers,
        &|r| abbr_rarete(r),
        &|n| cle_comparaison(n),
        &options_reelles(),
    );

    let applique = appliquer(&conn, &actions, &illustrations(&f.images)).expect("application");
    assert_eq!(applique, prevu, "rien n'a été refusé par les gardes");
    assert_eq!(applique.illustrations, f.bilan.illustrations);

    let obtenu = relire(&conn);
    assert_eq!(obtenu.len(), f.apres.len());
    for (obtenue, attendue) in obtenu.iter().zip(&f.apres) {
        assert_eq!(
            obtenue, attendue,
            "ligne {} ({} / {})",
            attendue.rowid, attendue.set_code, attendue.rarity
        );
    }
}

/// Rejouer la passe ne doit rien changer — c'est la garde d'idempotence.
///
/// Le compte, lui, **tombe à zéro** : `planifier` prévoit toujours ses 30
/// illustrations, mais l'`UPDATE` refuse les lignes déjà servies. C'est
/// exactement l'écart entre le bilan prévu et le bilan appliqué, et la seule
/// raison pour laquelle l'applicateur rend son propre bilan.
#[test]
fn rejouer_la_passe_ne_touche_plus_rien() {
    let racine = fixtures();
    let f: Fixture = lire(racine.join("oracle/artworks/LOCR-JP.json"));
    let capture: Capture = lire(racine.join("net/yugipedia/wikitext_LOCR-JP_rev5945579.json"));

    // On part de l'état d'APRÈS : la passe a déjà tourné.
    let conn = classeur_depuis(&f.apres);
    let (inventaire, _) = lire_inventaire(&conn).expect("inventaire");
    let (reference, libelles) = reference_depuis_wikitext(&capture.wikitext);
    let fichiers = fichiers_depuis_images(&f.images, "LOCR");
    let (actions, prevu) = planifier(
        &reference,
        &libelles,
        &inventaire,
        &fichiers,
        &|r| abbr_rarete(r),
        &|n| cle_comparaison(n),
        &options_reelles(),
    );
    assert_eq!(prevu.illustrations, 30, "le plan, lui, ne change pas");

    let applique = appliquer(&conn, &actions, &illustrations(&f.images)).expect("application");
    assert_eq!(applique.illustrations, 0, "toutes les lignes sont servies");
    assert_eq!(applique.ajoutes, 0);
    assert_eq!(applique.flags, 0);
    assert_eq!(relire(&conn), f.apres, "le classeur est inchangé");
}

#[test]
fn aucune_image_ne_sert_deux_fois() {
    // 54 tirages Overframe, 30 illustrés. Les 24 restants n'ont simplement
    // aucun fichier disponible — ce n'est pas la règle des fichiers nus qui
    // les écarte, contrairement à ce qu'on pourrait croire : elle est masquée
    // ici par le repli sur les autres raretés.
    let f: Fixture = lire(fixtures().join("oracle/artworks/LOCR-JP.json"));
    let overframe = f.avant.iter().filter(|l| l.extended_art != 0).count();
    assert_eq!(overframe, 54);
    assert_eq!(f.bilan.illustrations, 30);

    // Ce qui tient en revanche, et qui compte : aucune image externe ne se
    // retrouve sur deux lignes.
    let externes: Vec<&Option<String>> = f
        .apres
        .iter()
        .filter(|l| l.card_image_id.unwrap_or(0) < 0)
        .map(|l| &l.card_image_uuid)
        .collect();
    let uniques: std::collections::HashSet<_> = externes.iter().collect();
    assert_eq!(
        externes.len(),
        uniques.len(),
        "un fichier ne doit jamais servir deux fois"
    );
    assert_eq!(externes.len(), 30);
}
