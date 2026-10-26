// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les sets qu'on peut transformer en classeur.
//!
//! Portage de `creation_classeur_service.get_available_set_codes` et de
//! `_fetch_sets_from_cardinfo_db`.
//!
//! # Une seule requête, et non une par set
//!
//! Le décompte de cartes par set se fait par **agrégat**, pas par une requête
//! par ligne. `cardinfo.db` pèse 163 Mo et déclare plus de mille sets : le
//! schéma en N+1 ferait attendre l'ouverture de l'écran. Le Python le
//! documente comme tel, et c'est reproduit.
//!
//! # Deux requêtes tout de même, et pourquoi
//!
//! Les sets **TCG** sont agrégés sur le préfixe **nu** : `RA02-EN` et
//! `RA02-EU` sont le même classeur, `RA02`. Les sets **OCG japonais** gardent
//! leur suffixe — `LOCH-JP` et non `LOCH` — pour trois raisons que le Python
//! énumère : coexister avec le classeur TCG du même set, se distinguer à
//! l'œil dans la liste, et servir directement de nom de dossier.
//!
//! # L'écart : les classeurs déjà créés restent dans la liste
//!
//! Le Python les **retire**. On les garde, marqués — savoir qu'un set est déjà
//! chez soi est une information, et la faire disparaître oblige à aller
//! vérifier ailleurs. Ils ne sont simplement pas créables une seconde fois.
//!
//! # La locale anglaise sans préfixe — 2026-10-26
//!
//! YGOJSON ne donne **aucun** préfixe à la locale anglaise de plusieurs
//! boosters récents : celle de `INFO` (« The Infinite Forbidden ») range,
//! avec ses 125 tirages `INFO-EN…`, les cinq jetons `TKN5-EN006…010` glissés
//! dans ses paquets, et un préfixe commun n'existe plus. La requête TCG,
//! qui lit le préfixe de la locale, ne voyait donc pas le set : seul
//! `INFO-JP` était proposé, alors que le set existe en français. Mesuré sur
//! la base de l'utilisateur : `LEDE`, `INFO`, `SUDA`, `ALIN`, `DUAD`,
//! `DOOD`, `BPRO`, `DUOV`, `BACH`, `KC01` — dix sets, les six derniers
//! boosters compris.
//!
//! Pour une locale anglaise sans préfixe, le code se lit donc sur ses
//! **numéros** : celui que porte plus de la moitié d'entre eux (`INFO`, 101
//! numéros sur 106). Sans majorité — les cartes de participation, qui mêlent
//! cinquante préfixes — le set reste hors de la liste, comme avant ; et un
//! code qu'un set préfixé porte déjà reste le sien.

use rusqlite::Connection;
use ygo_core::paths::Paths;

use crate::error::Result;

/// Nombre minimum de caractères avant de filtrer.
///
/// `MIN_SEARCH_CHARS` du Python. En dessous, la liste reste vide : afficher
/// mille sets d'un coup ne rend service à personne.
pub const MIN_RECHERCHE: usize = 2;

/// Nombre maximum de résultats montrés.
///
/// `MAX_RESULTS` du Python. Au-delà, on demande d'affiner plutôt que de
/// construire une liste que personne ne parcourra.
pub const MAX_RESULTATS: usize = 50;

/// Un set, et ce qu'on en sait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetDisponible {
    /// Préfixe — `RA02`, `LOCH-JP`.
    pub code: String,
    /// Nom complet, dans la langue demandée.
    pub nom: String,
    /// Nombre de cartes distinctes.
    pub nb_cartes: usize,
    /// Un classeur existe-t-il déjà pour ce code ?
    pub deja_cree: bool,
}

/// Les sets déclarés par `cardinfo.db`, marqués de ceux qui ont déjà un
/// classeur.
///
/// Rend une liste **vide** si `cardinfo.db` est absente ou non initialisée —
/// pas une erreur : l'écran doit pouvoir s'ouvrir et le dire.
pub fn lister(paths: &Paths, francais: bool) -> Result<Vec<SetDisponible>> {
    let deja: std::collections::HashSet<String> = std::fs::read_dir(paths.classeurs())
        .map(|entrees| {
            entrees
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_uppercase())
                .collect()
        })
        .unwrap_or_default();

    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return Ok(Vec::new());
    };
    if !tables_presentes(&conn) {
        return Ok(Vec::new());
    }

    let mut sets = lire_sets(&conn, francais)?;
    for set in &mut sets {
        set.deja_cree = deja.contains(&set.code);
    }
    sets.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(sets)
}

/// Les trois tables nécessaires sont-elles là ?
///
/// Sans ce contrôle, une base non initialisée donne une erreur SQL
/// énigmatique (« no such table ») là où « la base n'est pas prête » est la
/// bonne réponse.
fn tables_presentes(conn: &Connection) -> bool {
    ["sets", "set_locales", "set_prints"].iter().all(|table| {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok()
    })
}

/// Requête TCG — préfixe **nu**, locales `en` et `eu` fusionnées.
const SQL_TCG: &str = "\
    SELECT SUBSTR(sl.prefix, 1, INSTR(sl.prefix || '-', '-') - 1) AS code, \
           s.name_en, s.name_fr, COUNT(DISTINCT sp.card_uuid) AS nb \
    FROM sets s \
    JOIN set_locales sl ON sl.set_uuid = s.uuid \
    LEFT JOIN set_prints sp ON sp.set_locale_id = sl.id \
    WHERE sl.prefix IS NOT NULL AND sl.prefix != '' \
      AND sl.language IN ('en', 'eu') \
    GROUP BY code, s.name_en, s.name_fr \
    HAVING code != ''";

/// Requête TCG des sets dont **aucune** locale `en`/`eu` n'a de préfixe — le
/// code est celui que porte plus de la moitié des **numéros** distincts de
/// la locale (cf. tête de module).
///
/// Les numéros, pas les tirages : la boîte « Yugi & Kaiba » range sept
/// tirages de `KACB-EN001` pour un seul de `YUCB-EN001` — deux cartes, deux
/// préfixes, aucune majorité. Un set dont une locale `en` ou `eu` a un
/// préfixe est laissé à [`SQL_TCG`]. Le décompte ne retient que les tirages
/// du code retenu — les jetons `TKN5` de `INFO` n'en font pas partie :
/// 101 cartes, comme YGOPRODeck.
const SQL_TCG_SANS_PREFIXE: &str = "\
    WITH tirages AS ( \
        SELECT sl.id AS locale, s.name_en AS name_en, s.name_fr AS name_fr, \
               sp.card_uuid AS carte, UPPER(sp.set_code) AS numero, \
               UPPER(SUBSTR(sp.set_code, 1, INSTR(sp.set_code || '-', '-') - 1)) AS code \
        FROM sets s \
        JOIN set_locales sl ON sl.set_uuid = s.uuid \
        JOIN set_prints sp ON sp.set_locale_id = sl.id \
        WHERE COALESCE(sl.prefix, '') = '' \
          AND sl.language IN ('en', 'eu') \
          AND COALESCE(sp.set_code, '') != '' \
          AND NOT EXISTS ( \
              SELECT 1 FROM set_locales autre \
              WHERE autre.set_uuid = s.uuid \
                AND autre.language IN ('en', 'eu') \
                AND COALESCE(autre.prefix, '') != '') \
    ), \
    comptes AS ( \
        SELECT locale, code, COUNT(DISTINCT numero) AS n, \
               SUM(COUNT(DISTINCT numero)) OVER (PARTITION BY locale) AS total \
        FROM tirages GROUP BY locale, code \
    ) \
    SELECT t.code, t.name_en, t.name_fr, COUNT(DISTINCT t.carte) AS nb \
    FROM tirages t \
    JOIN comptes c ON c.locale = t.locale AND c.code = t.code \
    WHERE c.n * 2 > c.total AND t.code != '' \
    GROUP BY t.code, t.name_en, t.name_fr";

/// Requête OCG japonaise — préfixe **complet**, suffixe conservé.
const SQL_OCG: &str = "\
    SELECT UPPER(sl.prefix) AS code, \
           s.name_en, s.name_fr, COUNT(DISTINCT sp.card_uuid) AS nb \
    FROM sets s \
    JOIN set_locales sl ON sl.set_uuid = s.uuid \
    LEFT JOIN set_prints sp ON sp.set_locale_id = sl.id \
    WHERE sl.prefix IS NOT NULL AND sl.prefix != '' \
      AND sl.language = 'jp' \
    GROUP BY code, s.name_en, s.name_fr \
    HAVING code != ''";

fn lire_sets(conn: &Connection, francais: bool) -> Result<Vec<SetDisponible>> {
    let mut sets = Vec::new();
    for sql in [SQL_TCG, SQL_OCG] {
        lire_requete(conn, sql, francais, &mut sets)?;
    }
    // Le complément ne doit jamais coûter la liste entière : une base où il
    // échouerait garde les sets des deux requêtes d'origine.
    let mut complement = Vec::new();
    match lire_requete(conn, SQL_TCG_SANS_PREFIXE, francais, &mut complement) {
        Ok(()) => {
            // Un code qu'un set préfixé porte déjà est le sien : la boîte
            // « Duelist Pack Collection Tin 2009 », dont la locale anglaise
            // range surtout des `RGBT-ENPP…`, ne doit pas se faire passer
            // pour « Raging Battle » — c'est lui que la création trouverait.
            let deja: std::collections::HashSet<String> =
                sets.iter().map(|s| s.code.clone()).collect();
            sets.extend(complement.into_iter().filter(|s| !deja.contains(&s.code)));
        }
        Err(e) => tracing::warn!(erreur = %e, "sets TCG sans préfixe anglais non lus"),
    }
    Ok(sets)
}

/// Ajoute à `sets` ce que rend une requête `(code, name_en, name_fr, nb)`.
fn lire_requete(
    conn: &Connection,
    sql: &str,
    francais: bool,
    sets: &mut Vec<SetDisponible>,
) -> Result<()> {
    let mut requete = conn.prepare(sql)?;
    let lignes = requete.query_map([], |l| {
        let code: String = l.get(0)?;
        let name_en: Option<String> = l.get(1)?;
        let name_fr: Option<String> = l.get(2)?;
        let nb: i64 = l.get(3)?;
        Ok((code, name_en, name_fr, nb))
    })?;
    for ligne in lignes {
        let (code, name_en, name_fr, nb) = ligne?;
        sets.push(SetDisponible {
            code,
            nom: nom_affiche(name_en.as_deref(), name_fr.as_deref(), francais),
            nb_cartes: nb.max(0) as usize,
            deja_cree: false,
        });
    }
    Ok(())
}

/// Le nom à montrer, selon la langue.
///
/// En français, le nom français d'abord, sinon l'anglais. En anglais,
/// l'anglais d'abord, sinon le français — c'est la cascade du Python, et elle
/// n'est **pas** symétrique par hasard : un set sans nom anglais est plus rare
/// qu'un set sans nom français.
#[must_use]
pub fn nom_affiche(name_en: Option<&str>, name_fr: Option<&str>, francais: bool) -> String {
    fn non_vide(v: Option<&str>) -> Option<&str> {
        v.filter(|s| !s.is_empty())
    }
    let (premier, second) = if francais {
        (non_vide(name_fr), non_vide(name_en))
    } else {
        (non_vide(name_en), non_vide(name_fr))
    };
    premier.or(second).unwrap_or_default().to_owned()
}

/// Ce qu'une recherche a trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resultats<'a> {
    /// Les sets retenus, au plus [`MAX_RESULTATS`].
    pub sets: Vec<&'a SetDisponible>,
    /// Nombre total de correspondances, avant plafonnement.
    pub total: usize,
    /// Le terme est-il trop court pour chercher ?
    pub trop_court: bool,
}

impl Resultats<'_> {
    /// La liste a-t-elle été tronquée ?
    #[must_use]
    pub fn tronquee(&self) -> bool {
        self.total > self.sets.len()
    }
}

/// Filtre les sets sur un terme.
///
/// Fonction **pure**. Le terme est comparé au code **et** au nom, sans casse.
/// En dessous de [`MIN_RECHERCHE`] caractères, rien n'est rendu — et le dire
/// est le rôle de [`Resultats::trop_court`], pas d'une liste vide qui
/// ressemblerait à « aucun résultat ».
#[must_use]
pub fn filtrer<'a>(sets: &'a [SetDisponible], terme: &str, max: usize) -> Resultats<'a> {
    let terme = terme.trim().to_lowercase();
    if terme.len() < MIN_RECHERCHE {
        return Resultats {
            sets: Vec::new(),
            total: 0,
            trop_court: true,
        };
    }
    let retenus: Vec<&SetDisponible> = sets
        .iter()
        .filter(|s| s.code.to_lowercase().contains(&terme) || s.nom.to_lowercase().contains(&terme))
        .collect();
    let total = retenus.len();
    Resultats {
        sets: retenus.into_iter().take(max).collect(),
        total,
        trop_court: false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn set(code: &str, nom: &str, nb: usize) -> SetDisponible {
        SetDisponible {
            code: code.to_owned(),
            nom: nom.to_owned(),
            nb_cartes: nb,
            deja_cree: false,
        }
    }

    fn jeu() -> Vec<SetDisponible> {
        vec![
            set("RA02", "25th Anniversary Rarity Collection II", 79),
            set("RA05", "Quarter Century Bonanza", 149),
            set("LOCH-JP", "Legacy of Destruction", 80),
            set("SDWD", "Structure Deck: White Dragon", 43),
        ]
    }

    #[test]
    fn un_terme_trop_court_ne_cherche_pas_et_le_dit() {
        let sets = jeu();
        for terme in ["", " ", "r", " r "] {
            let r = filtrer(&sets, terme, MAX_RESULTATS);
            assert!(r.trop_court, "{terme:?}");
            assert!(r.sets.is_empty());
            assert_eq!(r.total, 0);
        }
    }

    #[test]
    fn le_filtre_porte_sur_le_code_et_sur_le_nom() {
        let sets = jeu();
        assert_eq!(filtrer(&sets, "ra0", MAX_RESULTATS).sets.len(), 2);
        assert_eq!(
            filtrer(&sets, "RA0", MAX_RESULTATS).sets.len(),
            2,
            "sans casse"
        );
        assert_eq!(
            filtrer(&sets, "bonanza", MAX_RESULTATS).sets[0].code,
            "RA05",
            "sur le nom"
        );
        assert!(filtrer(&sets, "zzz", MAX_RESULTATS).sets.is_empty());
        assert!(!filtrer(&sets, "zzz", MAX_RESULTATS).trop_court);
    }

    /// Au-delà du plafond, la liste est coupée mais le **total** reste vrai :
    /// c'est lui qui permet de dire « affinez votre recherche ».
    #[test]
    fn au_dela_du_plafond_la_liste_est_coupee_mais_le_total_reste_vrai() {
        let sets: Vec<SetDisponible> = (0..80)
            .map(|i| set(&format!("SET{i:03}"), "Un set", 10))
            .collect();
        let r = filtrer(&sets, "set", 50);
        assert_eq!(r.sets.len(), 50);
        assert_eq!(r.total, 80);
        assert!(r.tronquee());

        let r = filtrer(&sets, "set000", 50);
        assert_eq!(r.total, 1);
        assert!(!r.tronquee());
    }

    #[test]
    fn le_nom_affiche_suit_la_langue_avec_repli() {
        assert_eq!(
            nom_affiche(Some("Anglais"), Some("Français"), true),
            "Français"
        );
        assert_eq!(
            nom_affiche(Some("Anglais"), Some("Français"), false),
            "Anglais"
        );
        // Repli quand la langue demandée n'a pas de nom.
        assert_eq!(nom_affiche(Some("Anglais"), None, true), "Anglais");
        assert_eq!(nom_affiche(None, Some("Français"), false), "Français");
        // Une chaîne vide compte comme absente.
        assert_eq!(nom_affiche(Some(""), Some("Français"), false), "Français");
        assert_eq!(nom_affiche(None, None, true), "");
    }

    // ── Lecture de cardinfo.db ──────────────────────────────────────────────

    fn cardinfo(chemin: &std::path::Path) {
        let conn = Connection::open(chemin).unwrap();
        conn.execute_batch(
            "CREATE TABLE sets (uuid TEXT PRIMARY KEY, name_en TEXT, name_fr TEXT);
             CREATE TABLE set_locales (id INTEGER PRIMARY KEY, set_uuid TEXT, \
                                       prefix TEXT, language TEXT);
             CREATE TABLE set_prints (id INTEGER PRIMARY KEY, set_locale_id INTEGER, \
                                      card_uuid TEXT, set_code TEXT);",
        )
        .unwrap();
        // RA02 : deux locales TCG du même set — elles doivent fusionner.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-ra02', 'Rarity Collection II', 'Collection Rareté II');
             INSERT INTO set_locales VALUES (1, 'u-ra02', 'RA02-EN', 'en');
             INSERT INTO set_locales VALUES (2, 'u-ra02', 'RA02-EU', 'eu');
             INSERT INTO set_prints VALUES (1, 1, 'c1', 'RA02-EN001');
             INSERT INTO set_prints VALUES (2, 1, 'c2', 'RA02-EN002');
             INSERT INTO set_prints VALUES (3, 2, 'c1', 'RA02-EU001');",
        )
        .unwrap();
        // LOCH : une locale TCG et une locale OCG-JP du même set.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-loch', 'Legacy of Destruction', NULL);
             INSERT INTO set_locales VALUES (3, 'u-loch', 'LOCH-EN', 'en');
             INSERT INTO set_locales VALUES (4, 'u-loch', 'LOCH-JP', 'jp');
             INSERT INTO set_prints VALUES (4, 3, 'd1', 'LOCH-EN001');
             INSERT INTO set_prints VALUES (5, 4, 'd1', 'LOCH-JP001');
             INSERT INTO set_prints VALUES (6, 4, 'd2', 'LOCH-JP002');",
        )
        .unwrap();
        // INFO, tel que YGOJSON le livre : la locale anglaise **sans
        // préfixe** — ses numéros `INFO-EN…` côtoient un jeton `TKN5-EN010`
        // glissé dans les paquets —, la française et la japonaise avec.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-info', 'The Infinite Forbidden', 'L''Infini Interdit');
             INSERT INTO set_locales VALUES (5, 'u-info', '', 'en');
             INSERT INTO set_locales VALUES (6, 'u-info', 'INFO-FR', 'fr');
             INSERT INTO set_locales VALUES (7, 'u-info', 'INFO-JP', 'jp');
             INSERT INTO set_prints VALUES (7, 5, 'e1', 'INFO-EN001');
             INSERT INTO set_prints VALUES (8, 5, 'e1', 'INFO-EN001');
             INSERT INTO set_prints VALUES (9, 5, 'e2', 'INFO-EN002');
             INSERT INTO set_prints VALUES (10, 5, 'jeton', 'TKN5-EN010');
             INSERT INTO set_prints VALUES (11, 6, 'e1', 'INFO-FR001');
             INSERT INTO set_prints VALUES (12, 7, 'e1', 'INFO-JP001');",
        )
        .unwrap();
        // Des cartes de participation : une locale anglaise sans préfixe
        // dont aucun préfixe de numéro n'a la majorité.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-sp', 'Sneak Peek Participation Cards', NULL);
             INSERT INTO set_locales VALUES (8, 'u-sp', NULL, 'en');
             INSERT INTO set_prints VALUES (13, 8, 'f1', 'STON-ENSP1');
             INSERT INTO set_prints VALUES (14, 8, 'f2', 'GLAS-ENSP1');
             INSERT INTO set_prints VALUES (15, 8, 'f3', 'LODT-ENSP1');",
        )
        .unwrap();
        // Une locale anglaise sans préfixe, mais une européenne avec : le
        // set passe par la requête d'origine, et une seule fois.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-mix', 'Mixed', NULL);
             INSERT INTO set_locales VALUES (9, 'u-mix', '', 'en');
             INSERT INTO set_locales VALUES (10, 'u-mix', 'MIX1-EU', 'eu');
             INSERT INTO set_prints VALUES (16, 9, 'g1', 'MIX1-EN001');
             INSERT INTO set_prints VALUES (17, 10, 'g1', 'MIX1-EU001');",
        )
        .unwrap();
        // Une boîte sans préfixe dont les numéros sont surtout ceux d'un set
        // préfixé (« Duelist Pack Collection Tin 2009 » et ses `RGBT-ENPP…`) :
        // le code appartient au set préfixé.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-tin', 'Collection Tin', NULL);
             INSERT INTO set_locales VALUES (11, 'u-tin', '', 'en');
             INSERT INTO set_prints VALUES (18, 11, 'h1', 'RA02-ENPP1');
             INSERT INTO set_prints VALUES (19, 11, 'h2', 'RA02-ENPP2');
             INSERT INTO set_prints VALUES (20, 11, 'h3', 'TIN1-EN001');",
        )
        .unwrap();
        // Une boîte à deux cartes : sept tirages de l'une, un de l'autre.
        // La majorité se compte en numéros — il n'y en a aucune.
        conn.execute_batch(
            "INSERT INTO sets VALUES ('u-box', 'Collector Box', NULL);
             INSERT INTO set_locales VALUES (12, 'u-box', '', 'en');
             INSERT INTO set_prints VALUES (21, 12, 'k1', 'KACB-EN001');
             INSERT INTO set_prints VALUES (22, 12, 'k1', 'KACB-EN001');
             INSERT INTO set_prints VALUES (23, 12, 'k1', 'KACB-EN001');
             INSERT INTO set_prints VALUES (24, 12, 'y1', 'YUCB-EN001');",
        )
        .unwrap();
    }

    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.classeurs()).unwrap();
        std::fs::create_dir_all(paths.cardinfo_db().parent().unwrap()).unwrap();
        cardinfo(&paths.cardinfo_db());
        (tmp, paths)
    }

    /// Les deux locales TCG d'un même set ne font qu'une entrée, et le
    /// décompte est celui des cartes **distinctes** — pas la somme des
    /// tirages.
    #[test]
    fn les_locales_tcg_d_un_meme_set_fusionnent() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, false).unwrap();
        let ra02: Vec<&SetDisponible> = sets.iter().filter(|s| s.code == "RA02").collect();
        assert_eq!(ra02.len(), 1, "une seule entrée pour RA02-EN et RA02-EU");
        assert_eq!(
            ra02[0].nb_cartes, 2,
            "c1 et c2, c1 n'est pas compté deux fois"
        );
        assert_eq!(ra02[0].nom, "Rarity Collection II");
    }

    /// La version japonaise garde son suffixe et coexiste avec la TCG : ce
    /// sont deux classeurs, et deux dossiers.
    #[test]
    fn la_version_japonaise_coexiste_avec_la_version_tcg() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, false).unwrap();
        let codes: Vec<&str> = sets.iter().map(|s| s.code.as_str()).collect();
        assert!(codes.contains(&"LOCH"), "la TCG, préfixe nu");
        assert!(codes.contains(&"LOCH-JP"), "l'OCG, suffixe gardé");
    }

    #[test]
    fn un_classeur_existant_est_marque_sans_disparaitre() {
        let (_tmp, paths) = installation();
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();

        let sets = lister(&paths, false).unwrap();
        let ra02 = sets.iter().find(|s| s.code == "RA02").unwrap();
        assert!(ra02.deja_cree, "marqué");
        assert!(
            sets.iter().any(|s| s.code == "LOCH"),
            "et les autres restent"
        );
    }

    #[test]
    fn le_nom_suit_la_langue_demandee() {
        let (_tmp, paths) = installation();
        let fr = lister(&paths, true).unwrap();
        let ra02 = fr.iter().find(|s| s.code == "RA02").unwrap();
        assert_eq!(ra02.nom, "Collection Rareté II");
        // LOCH n'a pas de nom français : repli sur l'anglais.
        let loch = fr.iter().find(|s| s.code == "LOCH").unwrap();
        assert_eq!(loch.nom, "Legacy of Destruction");
    }

    /// Une base absente ou non initialisée rend une liste vide, **pas une
    /// erreur** : l'écran doit s'ouvrir et l'expliquer.
    #[test]
    fn une_base_absente_ou_vide_rend_une_liste_vide_sans_erreur() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        assert!(lister(&paths, false).unwrap().is_empty(), "base absente");

        std::fs::create_dir_all(paths.cardinfo_db().parent().unwrap()).unwrap();
        Connection::open(paths.cardinfo_db()).unwrap();
        assert!(
            lister(&paths, false).unwrap().is_empty(),
            "base sans les tables"
        );
    }

    /// Le cas du 2026-10-26 : « The Infinite Forbidden » n'était proposé
    /// qu'en `INFO-JP`, sa locale anglaise n'ayant pas de préfixe. Le code se
    /// lit sur ses numéros — `INFO` en porte trois sur quatre, le jeton
    /// `TKN5` ne compte ni pour le code ni pour le nombre de cartes.
    #[test]
    fn un_set_sans_prefixe_anglais_est_propose_par_ses_numeros() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, true).unwrap();
        let info: Vec<&SetDisponible> = sets.iter().filter(|s| s.code == "INFO").collect();
        assert_eq!(info.len(), 1, "le set TCG est proposé, une fois");
        assert_eq!(info[0].nom, "L'Infini Interdit");
        assert_eq!(info[0].nb_cartes, 2, "e1 et e2 — pas le jeton");
        assert!(
            sets.iter().any(|s| s.code == "INFO-JP"),
            "la version japonaise reste proposée à côté"
        );
        assert!(
            !sets.iter().any(|s| s.code == "TKN5"),
            "le jeton n'est pas un set"
        );
    }

    /// Sans préfixe majoritaire, rien n'est deviné : les cartes de
    /// participation, qui mêlent les préfixes, restent hors de la liste — et
    /// la boîte à deux cartes aussi, quel que soit le nombre de tirages de
    /// l'une d'elles.
    #[test]
    fn sans_prefixe_majoritaire_le_set_reste_hors_de_la_liste() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, false).unwrap();
        for code in ["STON", "GLAS", "LODT", "KACB", "YUCB", ""] {
            assert!(!sets.iter().any(|s| s.code == code), "{code:?}");
        }
    }

    /// Une locale européenne préfixée suffit à la requête d'origine : le
    /// complément ne l'ajoute pas une seconde fois. Et un code qu'un set
    /// préfixé porte déjà reste le sien : la boîte qui range surtout des
    /// `RA02-ENPP…` ne devient pas un second « RA02 ».
    #[test]
    fn un_set_deja_propose_ne_l_est_pas_deux_fois() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, false).unwrap();
        assert_eq!(sets.iter().filter(|s| s.code == "MIX1").count(), 1);
        let ra02: Vec<&SetDisponible> = sets.iter().filter(|s| s.code == "RA02").collect();
        assert_eq!(ra02.len(), 1);
        assert_eq!(ra02[0].nom, "Rarity Collection II", "pas la boîte");
    }

    #[test]
    fn la_liste_sort_triee_par_code() {
        let (_tmp, paths) = installation();
        let sets = lister(&paths, false).unwrap();
        let codes: Vec<&str> = sets.iter().map(|s| s.code.as_str()).collect();
        let mut tries = codes.clone();
        tries.sort_unstable();
        assert_eq!(codes, tries);
    }
}
