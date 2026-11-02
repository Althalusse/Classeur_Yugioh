// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les sets que la base n'a pas encore, proposés via YGOPRODeck.
//!
//! # Pourquoi ce repli — 2026-11-01
//!
//! YGOJSON, d'où `cardinfo.db` est bâtie, est **figée au 2026-04-07**. Un set
//! sorti depuis n'y est pas, et le sélecteur, qui ne lit que la base, ne le
//! proposait pas : « Magnificent Monsters » (`MAMO`, 126 cartes, sorti le
//! 2026-09-04) était introuvable alors que YGOPRODeck le connaît. La
//! création, elle, savait déjà faire : un code absent de la base part par
//! YGOPRODeck (`creation::decider` → `DepuisApi`), puis passe par Yugipedia
//! comme tout classeur. Il manquait seulement de le **proposer**.
//!
//! # Ce que ce module retient
//!
//! Les sets de `cardsets.php` dont le code n'apparaît **nulle part** dans la
//! base — ni en préfixe de locale, ni en préfixe de numéro — et sortis en TCG
//! depuis [`DEPUIS`]. Mesuré le 2026-11-01 : sept sets sur 1 035 (`UP01`,
//! `WI26`, `TYP1`, `LAVD`, `MAMO`, `BETB`, `MAMS`). Les cinq absents plus
//! anciens (`CRC1`, `DDY1`, …) sont des cartes promotionnelles isolées que la
//! base range sous d'autres codes : les proposer ferait des doublons.
//!
//! # Pourquoi un fichier, et non une requête à l'ouverture
//!
//! La liste est calculée **pendant la MAJ BDD** et rangée dans
//! `bdd/sets_hors_base.json` — moins d'un kilo-octet. L'écran de création la
//! lit en une milliseconde et ne touche pas au réseau. Si le fichier manque
//! ou a plus de deux jours, l'écran montre d'abord la base, puis complète
//! la liste en arrière-plan ; et `cardsets.php` étant gardé deux jours dans
//! le cache HTTP, cela fait au plus une requête tous les deux jours — la
//! règle d'usage de YGOPRODeck est largement tenue.
//!
//! # Ce qui ne peut pas entrer en conflit
//!
//! Un code que la base porte n'est jamais repris d'ici, y compris quand la
//! base a été reconstruite après le calcul du fichier : le sélecteur filtre
//! une seconde fois. Une base illisible n'écrit **pas** de fichier — sinon
//! tous les sets passeraient pour absents.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ygo_core::paths::Paths;
use ygo_sources::ClientHttp;

use crate::error::{AppError, Result};

/// Nom du fichier, dans `bdd/`.
pub const FICHIER: &str = "sets_hors_base.json";

/// Date de gel de YGOJSON : seuls les sets sortis depuis sont retenus.
pub const DEPUIS: &str = "2026-04-07";

/// Âge au-delà duquel le fichier est recalculé — la durée de cache de
/// `cardsets.php`, pour ne jamais demander plus souvent que le cache ne
/// garde.
pub const PEREMPTION: Duration = ygo_sources::cache::DUREE_YGOPRODECK;

/// Un set connu de YGOPRODeck et absent de la base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetHorsBase {
    /// Préfixe nu — `MAMO`.
    pub code: String,
    /// Nom anglais, tel que YGOPRODeck l'écrit.
    pub nom: String,
    /// Nombre de cartes annoncé.
    pub nb_cartes: usize,
    /// Sortie TCG, `AAAA-MM-JJ`.
    pub date: String,
}

/// Le contenu du fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fichier {
    /// Format du fichier.
    version: u32,
    /// La date de gel appliquée.
    depuis: String,
    /// Les sets retenus, triés par code.
    sets: Vec<SetHorsBase>,
}

/// Une entrée de `cardsets.php`, lue sans exigence : une entrée mal formée
/// ne doit pas coûter la liste entière.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SetCatalogue {
    /// Préfixe — `MAMO`.
    #[serde(default)]
    pub set_code: String,
    /// Nom complet.
    #[serde(default)]
    pub set_name: String,
    /// Nombre de cartes, s'il est donné.
    #[serde(default)]
    pub num_of_cards: serde_json::Value,
    /// Sortie TCG, `AAAA-MM-JJ`, si elle est connue.
    #[serde(default)]
    pub tcg_date: Option<String>,
}

/// Chemin du fichier.
#[must_use]
pub fn chemin(paths: &Paths) -> PathBuf {
    paths.bdd().join(FICHIER)
}

/// Une date `AAAA-MM-JJ` bien formée ?
fn date_valide(date: &str) -> bool {
    let octets = date.as_bytes();
    octets.len() == 10
        && octets.iter().enumerate().all(|(i, o)| match i {
            4 | 7 => *o == b'-',
            _ => o.is_ascii_digit(),
        })
}

/// Les sets du catalogue à proposer. Fonction **pure**.
///
/// Retient un set si son code est absent de `codes_base`, s'il a des cartes
/// et s'il est sorti en TCG le jour de `depuis` ou après — y compris les sets
/// annoncés à une date future, dont YGOPRODeck publie déjà la liste. Un code
/// présent deux fois garde l'entrée la plus fournie.
#[must_use]
pub fn retenir(
    catalogue: &[SetCatalogue],
    codes_base: &HashSet<String>,
    depuis: &str,
) -> Vec<SetHorsBase> {
    let mut retenus: HashMap<String, SetHorsBase> = HashMap::new();
    for entree in catalogue {
        let code = entree.set_code.trim().to_uppercase();
        if code.is_empty() || code.contains('-') || codes_base.contains(&code) {
            continue;
        }
        let Some(date) = entree.tcg_date.as_deref().map(str::trim) else {
            continue;
        };
        if !date_valide(date) || date < depuis {
            continue;
        }
        let nb = entree
            .num_of_cards
            .as_u64()
            .or_else(|| {
                entree
                    .num_of_cards
                    .as_str()
                    .and_then(|s| s.trim().parse().ok())
            })
            .unwrap_or(0);
        let nom = entree.set_name.trim();
        if nb == 0 || nom.is_empty() {
            continue;
        }
        let set = SetHorsBase {
            code: code.clone(),
            nom: nom.to_owned(),
            nb_cartes: usize::try_from(nb).unwrap_or(usize::MAX),
            date: date.to_owned(),
        };
        match retenus.get(&code) {
            Some(deja) if deja.nb_cartes >= set.nb_cartes => {}
            _ => {
                retenus.insert(code, set);
            }
        }
    }
    let mut sets: Vec<SetHorsBase> = retenus.into_values().collect();
    sets.sort_by(|a, b| a.code.cmp(&b.code));
    sets
}

/// Tous les codes que la base porte : préfixes de locale **et** préfixes de
/// numéro, nus et en capitales.
///
/// Les deux sources, parce que YGOJSON laisse sans préfixe la locale anglaise
/// de plusieurs boosters (cf. `selecteur`) : seuls leurs numéros disent
/// qu'ils sont là.
///
/// # Errors
///
/// Si la base ne se lit pas.
pub fn codes_de_la_base(conn: &Connection) -> Result<HashSet<String>> {
    const SQL: &str = "\
        SELECT UPPER(SUBSTR(prefix, 1, INSTR(prefix || '-', '-') - 1)) \
        FROM set_locales WHERE COALESCE(prefix, '') != '' \
        UNION \
        SELECT UPPER(SUBSTR(set_code, 1, INSTR(set_code || '-', '-') - 1)) \
        FROM set_prints WHERE COALESCE(set_code, '') != ''";
    let mut requete = conn.prepare(SQL)?;
    let codes = requete
        .query_map([], |l| l.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<String>, _>>()?;
    Ok(codes.into_iter().filter(|c| !c.is_empty()).collect())
}

/// Les sets rangés dans le fichier. Vide s'il manque ou ne se lit pas : le
/// sélecteur montre alors la base seule, comme avant.
#[must_use]
pub fn lire(paths: &Paths) -> Vec<SetHorsBase> {
    std::fs::read(chemin(paths))
        .ok()
        .and_then(|octets| serde_json::from_slice::<Fichier>(&octets).ok())
        .map(|f| f.sets)
        .unwrap_or_default()
}

/// Le fichier doit-il être recalculé ?
///
/// Oui s'il manque, ne se lit pas, a dépassé [`PEREMPTION`], est plus ancien
/// que `cardinfo.db` — la base a été reconstruite sans que le calcul suive —
/// ou a été fait avec une autre date de gel.
#[must_use]
pub fn a_rafraichir(paths: &Paths) -> bool {
    let fichier = chemin(paths);
    let Ok(meta) = std::fs::metadata(&fichier) else {
        return true;
    };
    let Ok(modifie) = meta.modified() else {
        return true;
    };
    let perime = SystemTime::now()
        .duration_since(modifie)
        .is_ok_and(|age| age > PEREMPTION);
    let base_plus_recente = std::fs::metadata(paths.cardinfo_db())
        .and_then(|m| m.modified())
        .is_ok_and(|base| base > modifie);
    let lisible = std::fs::read(&fichier)
        .ok()
        .and_then(|o| serde_json::from_slice::<Fichier>(&o).ok())
        .is_some_and(|f| f.version == 1 && f.depuis == DEPUIS);
    perime || base_plus_recente || !lisible
}

/// Écrit le fichier, d'un renommage.
///
/// # Errors
///
/// Si `bdd/` n'accepte pas l'écriture.
pub fn ecrire(paths: &Paths, sets: &[SetHorsBase]) -> Result<()> {
    let dossier = paths.bdd();
    std::fs::create_dir_all(&dossier)?;
    let contenu = Fichier {
        version: 1,
        depuis: DEPUIS.to_owned(),
        sets: sets.to_vec(),
    };
    let octets = serde_json::to_vec_pretty(&contenu)?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dossier)?;
    std::io::Write::write_all(&mut tmp, &octets)?;
    tmp.persist(chemin(paths)).map_err(|e| e.error)?;
    Ok(())
}

/// Calcule la liste d'après un catalogue déjà lu, et l'écrit.
///
/// # Errors
///
/// Si la base ne s'ouvre pas, ne déclare aucun code — tous les sets
/// passeraient pour absents —, ou si le fichier ne s'écrit pas.
pub fn calculer(paths: &Paths, catalogue: &[SetCatalogue]) -> Result<Vec<SetHorsBase>> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())
        .map_err(|e| AppError::Creation(format!("cardinfo.db illisible : {e}")))?;
    let codes = codes_de_la_base(&conn)?;
    if codes.is_empty() {
        return Err(AppError::Creation(
            "cardinfo.db ne déclare aucun set — liste des sets hors base non calculée".to_owned(),
        ));
    }
    let sets = retenir(catalogue, &codes, DEPUIS);
    ecrire(paths, &sets)?;
    Ok(sets)
}

/// Relit `cardsets.php` (cache HTTP de deux jours) et recalcule le fichier.
///
/// # Errors
///
/// Si YGOPRODeck ne répond pas, ou pour les raisons de [`calculer`]. Le
/// fichier précédent reste alors en place.
pub async fn rafraichir(paths: &Paths, client: &ClientHttp) -> Result<Vec<SetHorsBase>> {
    let catalogue: Vec<SetCatalogue> = ygo_sources::ygoprodeck::lister_sets(client)
        .await
        .map_err(|e| AppError::Creation(format!("liste des sets YGOPRODeck : {e}")))?;
    let sets = calculer(paths, &catalogue)?;
    tracing::info!(
        sets = sets.len(),
        catalogue = catalogue.len(),
        depuis = DEPUIS,
        "sets hors base : liste recalculée"
    );
    Ok(sets)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn entree(code: &str, nom: &str, nb: serde_json::Value, date: Option<&str>) -> SetCatalogue {
        SetCatalogue {
            set_code: code.to_owned(),
            set_name: nom.to_owned(),
            num_of_cards: nb,
            tcg_date: date.map(str::to_owned),
        }
    }

    fn base(codes: &[&str]) -> HashSet<String> {
        codes.iter().map(|c| (*c).to_owned()).collect()
    }

    /// Le cas qui a motivé le module : `MAMO`, sorti après le gel et absent
    /// de la base, est retenu ; `RA05`, que la base porte, ne l'est pas.
    #[test]
    fn un_set_recent_absent_de_la_base_est_retenu() {
        let catalogue = vec![
            entree(
                "MAMO",
                "Magnificent Monsters",
                126.into(),
                Some("2026-09-04"),
            ),
            entree(
                "RA05",
                "Quarter Century Bonanza",
                149.into(),
                Some("2026-05-01"),
            ),
        ];
        let sets = retenir(&catalogue, &base(&["RA05"]), DEPUIS);
        assert_eq!(
            sets,
            vec![SetHorsBase {
                code: "MAMO".to_owned(),
                nom: "Magnificent Monsters".to_owned(),
                nb_cartes: 126,
                date: "2026-09-04".to_owned(),
            }]
        );
    }

    /// Les absents d'avant le gel sont des promotionnelles que la base range
    /// ailleurs ; un set sans date, sans carte ou sans nom n'est pas
    /// créable. Le jour du gel lui-même compte.
    #[test]
    fn les_sets_anciens_vides_ou_sans_date_sont_ecartes() {
        let catalogue = vec![
            entree("DDY1", "Old promo", 1.into(), Some("2019-01-01")),
            entree("CRC1", "Crocs collaboration card", 1.into(), None),
            entree("VIDE", "Empty", 0.into(), Some("2026-06-01")),
            entree("SANS", "", 10.into(), Some("2026-06-01")),
            entree("MAUV", "Bad date", 10.into(), Some("2026/06/01")),
            entree("GEL1", "Freeze day", 5.into(), Some("2026-04-07")),
            entree("FUTR", "Announced", 60.into(), Some("2027-01-15")),
        ];
        let codes: Vec<String> = retenir(&catalogue, &base(&[]), DEPUIS)
            .into_iter()
            .map(|s| s.code)
            .collect();
        assert_eq!(codes, vec!["FUTR", "GEL1"]);
    }

    /// Un code en double garde l'entrée la plus fournie ; la casse et les
    /// espaces ne font pas deux sets ; un nombre écrit en texte se lit.
    #[test]
    fn les_doublons_et_les_formats_laches_sont_tolerés() {
        let catalogue = vec![
            entree("lavd", " Light and Vapor ", "30".into(), Some("2026-07-01")),
            entree(" LAVD ", "Light and Vapor", 45.into(), Some("2026-07-01")),
            entree("LAVD", "Light and Vapor", 12.into(), Some("2026-07-01")),
            entree("PRE-EN", "Not a prefix", 3.into(), Some("2026-07-01")),
        ];
        let sets = retenir(&catalogue, &base(&[]), DEPUIS);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].code, "LAVD");
        assert_eq!(sets[0].nb_cartes, 45);
    }

    /// Une entrée de `cardsets.php` à laquelle il manque des champs se lit
    /// quand même : la réponse entière ne doit pas échouer pour elle.
    #[test]
    fn une_reponse_incomplete_se_lit() {
        let json = r#"[{"set_name":"A","set_code":"AAAA","num_of_cards":3,"tcg_date":"2026-08-01"},
                       {"set_name":"Crocs"},
                       {"set_code":"BBBB","num_of_cards":"7","tcg_date":"2026-08-02","set_image":"x"}]"#;
        let catalogue: Vec<SetCatalogue> = serde_json::from_str(json).unwrap();
        assert_eq!(catalogue.len(), 3);
        let codes: Vec<String> = retenir(&catalogue, &base(&[]), DEPUIS)
            .into_iter()
            .map(|s| s.code)
            .collect();
        assert_eq!(codes, vec!["AAAA"], "BBBB n'a pas de nom");
    }

    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        let conn = Connection::open(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_locales (id INTEGER PRIMARY KEY, set_uuid TEXT, \
                                       prefix TEXT, language TEXT);
             CREATE TABLE set_prints (id INTEGER PRIMARY KEY, set_locale_id INTEGER, \
                                      card_uuid TEXT, set_code TEXT);
             INSERT INTO set_locales VALUES (1, 'a', 'RA05-EN', 'en');
             INSERT INTO set_locales VALUES (2, 'b', '', 'en');
             INSERT INTO set_locales VALUES (3, 'c', NULL, 'en');
             INSERT INTO set_prints VALUES (1, 2, 'x', 'info-EN001');
             INSERT INTO set_prints VALUES (2, 3, 'y', NULL);",
        )
        .unwrap();
        (tmp, paths)
    }

    /// Les codes de la base se lisent sur les préfixes **et** sur les
    /// numéros : `INFO`, dont la locale anglaise n'a pas de préfixe, compte.
    #[test]
    fn les_codes_de_la_base_viennent_des_prefixes_et_des_numeros() {
        let (_tmp, paths) = installation();
        let conn = Connection::open(paths.cardinfo_db()).unwrap();
        let codes = codes_de_la_base(&conn).unwrap();
        assert_eq!(codes, base(&["RA05", "INFO"]));
    }

    #[test]
    fn le_calcul_ecrit_un_fichier_que_la_lecture_rend() {
        let (_tmp, paths) = installation();
        assert!(lire(&paths).is_empty(), "pas de fichier, pas de set");
        assert!(a_rafraichir(&paths), "un fichier absent est à calculer");

        let catalogue = vec![
            entree(
                "MAMO",
                "Magnificent Monsters",
                126.into(),
                Some("2026-09-04"),
            ),
            entree(
                "INFO",
                "The Infinite Forbidden",
                101.into(),
                Some("2026-07-01"),
            ),
        ];
        let sets = calculer(&paths, &catalogue).unwrap();
        assert_eq!(sets.len(), 1, "INFO est dans la base par ses numéros");
        assert_eq!(lire(&paths), sets);
        assert!(
            !a_rafraichir(&paths),
            "un fichier frais ne se recalcule pas"
        );
    }

    /// Une base vide n'écrit rien : sans quoi tout le catalogue passerait
    /// pour absent, et le sélecteur proposerait mille doublons.
    #[test]
    fn une_base_vide_n_ecrit_pas_de_fichier() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        let conn = Connection::open(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_locales (id INTEGER, set_uuid TEXT, prefix TEXT, language TEXT);
             CREATE TABLE set_prints (id INTEGER, set_locale_id INTEGER, card_uuid TEXT, \
                                      set_code TEXT);",
        )
        .unwrap();
        drop(conn);
        let catalogue = vec![entree("MAMO", "M", 1.into(), Some("2026-09-04"))];
        assert!(calculer(&paths, &catalogue).is_err());
        assert!(!chemin(&paths).exists());

        let absente = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(absente.path());
        assert!(calculer(&paths, &catalogue).is_err(), "base absente");
        assert!(!chemin(&paths).exists());
    }

    /// Un fichier illisible ou d'une autre date de gel se recalcule, et se
    /// lit vide en attendant.
    #[test]
    fn un_fichier_illisible_ou_d_un_autre_gel_se_recalcule() {
        let (_tmp, paths) = installation();
        std::fs::write(chemin(&paths), b"{pas du json").unwrap();
        assert!(lire(&paths).is_empty());
        assert!(a_rafraichir(&paths));

        std::fs::write(
            chemin(&paths),
            br#"{"version":1,"depuis":"2025-01-01","sets":[]}"#,
        )
        .unwrap();
        assert!(a_rafraichir(&paths));
    }
}
