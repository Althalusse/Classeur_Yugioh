// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Migration des bases de classeur — `ensure_columns`.
//!
//! Portage **à l'identique** de `module/db_migrations.py`.
//!
//! Deux générations de classeurs coexistent chez l'utilisateur :
//!
//! - **G1** (« YGOJSON ») : `card_uuid`, `card_image_uuid`, `card_image_url`,
//!   `card_image_id` ;
//! - **G2** (« YGOPRODeck ») : ajoute `sort_order`, `rarity_code`, `card_type`,
//!   `atk`, `def_val`, `level`, `attribute`, `race`, et les colonnes de
//!   possession.
//!
//! Au démarrage, chaque classeur existant est ouvert et les colonnes manquantes
//! sont ajoutées par `ALTER TABLE`. **L'ordre d'ajout compte** : il détermine
//! l'ordre des colonnes d'une base migrée. Il est donc repris exactement dans
//! l'ordre du dictionnaire Python.
//!
//! Constat sur les données réelles : les 17 classeurs de la V1.0.3 sont déjà
//! tous en 24 colonnes (`Migrations classeurs : 17 OK, 0 échec(s)` au journal).
//! La migration G1 → G2 n'a donc plus d'exemplaire vivant à observer — raison de
//! plus pour la porter mot à mot plutôt que de l'adapter.

use rusqlite::Connection;

use crate::error::Result;

/// Colonnes applicatives requises, **dans l'ordre d'ajout du Python**.
///
/// Reprend `db_migrations._REQUIRED_COLUMNS`. Un dictionnaire Python conserve
/// son ordre d'insertion : ce tableau doit rester dans cet ordre-là.
pub const COLONNES_REQUISES: [(&str, &str); 16] = [
    // Colonnes G1 (possession)
    ("possessed", "INTEGER DEFAULT 0"),
    ("quantite", "INTEGER DEFAULT 0"),
    ("qualite", "TEXT DEFAULT NULL"),
    ("is_custom", "INTEGER DEFAULT 0"),
    // Colonnes G2 (API YGOPRODeck)
    ("sort_order", "INTEGER DEFAULT 0"),
    ("rarity_code", "TEXT DEFAULT ''"),
    ("card_image_small", "TEXT DEFAULT ''"),
    ("card_type", "TEXT DEFAULT ''"),
    ("atk", "INTEGER"),
    ("def_val", "INTEGER"),
    ("level", "INTEGER"),
    ("attribute", "TEXT DEFAULT ''"),
    ("race", "TEXT DEFAULT ''"),
    // name_fr pour les classeurs G1 qui ne l'auraient pas
    ("name_fr", "TEXT DEFAULT ''"),
    // Édition Scanflip, pour le round-trip CSV.
    // Valeurs : '1st' / 'unlimited' / 'limited' / NULL (non spécifiée).
    ("edition", "TEXT DEFAULT NULL"),
    // Overframe (art étendu OCG) : 1 = tirage Overframe, 0 = cadre normal.
    // Traité comme un artwork distinct par le tri et le filtre N raretés.
    ("extended_art", "INTEGER DEFAULT 0"),
];

/// Ajoute à `table` les colonnes de [`COLONNES_REQUISES`] qui manquent.
///
/// Retourne la liste des colonnes ajoutées, dans l'ordre où elles l'ont été.
///
/// Comme en Python : si la table n'existe pas, la fonction ne fait **rien** et
/// retourne une liste vide — ce n'est pas une erreur.
pub fn ensure_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    if !table_existe(conn, table)? {
        return Ok(Vec::new());
    }

    let existantes = colonnes(conn, table)?;
    let mut ajoutees = Vec::new();

    for (nom, definition) in COLONNES_REQUISES {
        if existantes.iter().any(|c| c == nom) {
            continue;
        }
        // `table` et `nom` ne viennent jamais d'une saisie utilisateur : ce
        // sont des constantes du programme et un nom de table interne. SQLite
        // n'accepte pas de paramètre lié dans un DDL.
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {nom} {definition}"),
            [],
        )?;
        ajoutees.push(nom.to_owned());
    }
    Ok(ajoutees)
}

/// Applique [`ensure_columns`] à la table `cards`, cas d'usage courant.
pub fn migrer_classeur(conn: &Connection) -> Result<Vec<String>> {
    ensure_columns(conn, "cards")
}

fn table_existe(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Noms des colonnes d'une table, dans l'ordre du schéma.
pub fn colonnes(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut requete = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let noms = requete
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(noms)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::schema::{creer_tout, DDL_CLASSEUR};

    /// Schéma d'un classeur de première génération, tel que le décrit
    /// `db_migrations.py` : les colonnes de possession et toutes les colonnes
    /// YGOPRODeck manquent.
    const DDL_G1: &str = "CREATE TABLE cards (
        card_uuid       TEXT,
        card_image_uuid TEXT,
        card_image_id   INTEGER,
        set_code        TEXT,
        rarity          TEXT,
        set_name        TEXT,
        name            TEXT,
        card_image_url  TEXT
    )";

    fn base_g1() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(DDL_G1, []).unwrap();
        conn
    }

    #[test]
    fn migration_g1_vers_g2_ajoute_les_16_colonnes_dans_l_ordre() {
        let conn = base_g1();
        let ajoutees = ensure_columns(&conn, "cards").unwrap();

        let attendues: Vec<&str> = COLONNES_REQUISES.iter().map(|(n, _)| *n).collect();
        assert_eq!(ajoutees, attendues);

        // La table de départ avait 8 colonnes, aucune en commun avec les 16.
        assert_eq!(colonnes(&conn, "cards").unwrap().len(), 8 + 16);
    }

    #[test]
    fn migration_est_idempotente() {
        let conn = base_g1();
        assert_eq!(ensure_columns(&conn, "cards").unwrap().len(), 16);
        assert!(
            ensure_columns(&conn, "cards").unwrap().is_empty(),
            "une deuxième passe ne doit rien ajouter"
        );
    }

    #[test]
    fn un_classeur_g2_neuf_n_a_rien_a_migrer() {
        let conn = Connection::open_in_memory().unwrap();
        creer_tout(&conn, DDL_CLASSEUR).unwrap();
        assert!(
            migrer_classeur(&conn).unwrap().is_empty(),
            "le DDL de référence contient déjà les 16 colonnes requises"
        );
    }

    #[test]
    fn table_absente_ne_fait_rien_et_n_est_pas_une_erreur() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(ensure_columns(&conn, "cards").unwrap().is_empty());
    }

    #[test]
    fn les_donnees_existantes_survivent_a_la_migration() {
        let conn = base_g1();
        conn.execute(
            "INSERT INTO cards (card_uuid, set_code, rarity, name) VALUES (?1, ?2, ?3, ?4)",
            [
                "uuid-1",
                "RA05-EN134",
                "Ultra Rare",
                "Blue-Eyes White Dragon",
            ],
        )
        .unwrap();

        ensure_columns(&conn, "cards").unwrap();

        let (code, nom, possede): (String, String, i64) = conn
            .query_row(
                "SELECT set_code, name, possessed FROM cards WHERE card_uuid = 'uuid-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(code, "RA05-EN134");
        assert_eq!(nom, "Blue-Eyes White Dragon");
        assert_eq!(possede, 0, "la valeur par défaut de la colonne ajoutée");
    }

    #[test]
    fn migration_partielle_n_ajoute_que_ce_qui_manque() {
        let conn = base_g1();
        conn.execute(
            "ALTER TABLE cards ADD COLUMN possessed INTEGER DEFAULT 0",
            [],
        )
        .unwrap();
        conn.execute(
            "ALTER TABLE cards ADD COLUMN extended_art INTEGER DEFAULT 0",
            [],
        )
        .unwrap();

        let ajoutees = ensure_columns(&conn, "cards").unwrap();
        assert_eq!(ajoutees.len(), 14);
        assert!(!ajoutees.iter().any(|c| c == "possessed"));
        assert!(!ajoutees.iter().any(|c| c == "extended_art"));
    }
}
