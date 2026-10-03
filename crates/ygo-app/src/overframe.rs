// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Complétion des tirages Overframe depuis Yugipedia.
//!
//! Portage de `module/donnees/overframe_enrichment.py`.
//!
//! # Le problème
//!
//! YGOJSON est **incomplète** sur les sets à traitement Overframe. Pour une
//! carte chase comme `LOCR-JP001`, elle ne liste que les tirages de base là où
//! la boîte en contient cinq ; les variantes *extended art* manquent. Pire, elle
//! ne distingue pas le cadre : une même rareté peut exister en cadre normal
//! **et** en Overframe, et YGOJSON n'en voit qu'une.
//!
//! L'application place les cartes séquentiellement dans le classeur. Un tirage
//! manquant ne fait donc pas qu'un trou : il **décale toute la grille**.
//!
//! # L'Overframe n'est pas global
//!
//! Dans LOCH et LOCR, l'extended art n'existe que sur un sous-ensemble de
//! cartes chase — Yugipedia le dit pour LOCH : *« 18 Prismatic Secret Rares are
//! only available with an extended art, and these extended art cards are also
//! available as Ultra Rare and Grand Master Rare »*. L'immense majorité des
//! cartes n'a **aucun** tirage Overframe, et la détection ne doit jamais en
//! marquer une par défaut.
//!
//! # Additive, jamais destructive
//!
//! La réconciliation n'efface rien. Pour chaque `(numéro, rareté)` déclaré
//! Overframe par Yugipedia :
//!
//! | situation | action |
//! |---|---|
//! | déjà présent en `extended_art = 1` | rien |
//! | rareté présente dans les **deux** cadres | **ajouter** la ligne Overframe, garder la normale |
//! | rareté Overframe seulement, importée à tort en cadre normal | **corriger** le drapeau, sans doublon |
//! | absente de la base | **ajouter** la ligne |
//!
//! Les métadonnées — `card_uuid`, `card_image_uuid`, `set_uuid`,
//! `set_locale_id`, `edition`, `qty`, `print_image_url` — sont **héritées**
//! d'un tirage existant du même numéro : tous les tirages d'une carte partagent
//! ses références.
//!
//! # Ce que ça vaut en chiffres
//!
//! Sur l'installation de référence, `LOCR-JP` passe de **282 à 318** tirages :
//! 36 lignes ajoutées, 18 drapeaux corrigés, 54 tirages Overframe au total.
//! C'est exactement l'écart que `ygo-cli comparer` signalait entre la base
//! construite par le Rust et celle de la V1.0.4.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use ygo_sources::yugipedia::{self, EntreeSetList};
use ygo_sources::ClientHttp;

use crate::error::{AppError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Sets connus
// ─────────────────────────────────────────────────────────────────────────────

/// Un set à traitement Overframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetOverframe {
    /// Préfixe des codes de set, `LOCR-JP`.
    pub prefixe: &'static str,
    /// Nom anglais, tel que Yugipedia le titre.
    pub nom_anglais: &'static str,
    /// Région de la page, `OCG-JP`.
    pub region: &'static str,
}

/// Sets à traitement Overframe connus (`OVERFRAME_SETS`).
///
/// La liste est volontairement courte : le quota Yugipedia est d'une requête
/// par seconde, et parcourir les 3 300 sets prendrait près d'une heure pour
/// deux résultats. Les sets absents de cette liste sont couverts au coup par
/// coup, à la création d'un classeur.
pub const SETS_OVERFRAME: [SetOverframe; 2] = [
    SetOverframe {
        prefixe: "LOCH-JP",
        nom_anglais: "Limit Over Collection: The Heroes",
        region: "OCG-JP",
    },
    SetOverframe {
        prefixe: "LOCR-JP",
        nom_anglais: "Limit Over Collection: The Rivals",
        region: "OCG-JP",
    },
];

/// Mots-clés signalant un tirage extended art dans la note Yugipedia
/// (`_EXTENDED_KEYWORDS`). Comparés en minuscules.
///
/// `over flame` n'est pas une coquille de ce portage : la faute figure telle
/// quelle dans le Python, et donc probablement dans un wikitext rencontré.
pub const MOTS_CLES_EXTENDED: [&str; 5] = [
    "extended art",
    "extended artwork",
    "overframe",
    "over frame",
    "over flame",
];

/// Raretés qui n'existent **qu'en** Overframe (`_ALWAYS_EXTENDED`).
///
/// Filet de sécurité, et le signal le plus fiable des trois : la Grand Master
/// Rare n'existe pas en cadre normal, donc sa seule présence classe l'entrée —
/// y compris quand la mention « extended art » manque du wikitext, ce qui
/// arrive.
pub const RARETES_TOUJOURS_EXTENDED: [&str; 2] = ["grand master rare", "grandmaster rare"];

// ─────────────────────────────────────────────────────────────────────────────
// Classification — logique pure
// ─────────────────────────────────────────────────────────────────────────────

/// L'entrée Yugipedia décrit-elle des tirages Overframe ?
///
/// Portage de `_entree_est_overframe`. Trois signaux, réunis par un OU :
/// le drapeau posé par le parser, un mot-clé dans la note, ou une rareté
/// exclusivement Overframe. Quand une entrée est Overframe, **toutes** ses
/// raretés le sont.
pub fn entree_est_overframe(entree: &EntreeSetList, raretes: &[String]) -> bool {
    if entree.extended_art {
        return true;
    }
    let note = entree.note.to_lowercase();
    if MOTS_CLES_EXTENDED.iter().any(|k| note.contains(k)) {
        return true;
    }
    raretes
        .iter()
        .any(|r| RARETES_TOUJOURS_EXTENDED.contains(&r.to_lowercase().as_str()))
}

/// Répartition des `(numéro, rareté)` d'une Set list entre les deux cadres.
///
/// Un même couple peut figurer **dans les deux** : c'est le cas des Prismatic
/// Secret Rare des cartes chase, qui existent en cadre normal et en Overframe.
/// C'est aussi ce qui distingue « ajouter une ligne » de « corriger un
/// drapeau ».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classement {
    /// Tirages en cadre normal.
    pub normal: BTreeSet<(String, String)>,
    /// Tirages en cadre Overframe.
    pub extended: BTreeSet<(String, String)>,
}

/// Classe les entrées d'une Set list.
///
/// Portage de `_classify_prints`. Une entrée sans aucune rareté exploitable est
/// ignorée — elle ne décrit aucun tirage.
pub fn classer(entrees: &[EntreeSetList]) -> Classement {
    let mut classement = Classement::default();
    for entree in entrees {
        let raretes: Vec<String> = entree
            .raretes
            .iter()
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty())
            .collect();
        if raretes.is_empty() {
            continue;
        }
        let cible = if entree_est_overframe(entree, &raretes) {
            &mut classement.extended
        } else {
            &mut classement.normal
        };
        for rarete in raretes {
            cible.insert((entree.numero.clone(), rarete));
        }
    }
    classement
}

// ─────────────────────────────────────────────────────────────────────────────
// Schéma
// ─────────────────────────────────────────────────────────────────────────────

/// Ajoute `set_prints.extended_art` si la colonne manque.
///
/// Portage de `ensure_extension_art_column`. Sans objet sur une base construite
/// par ce portage — le DDL de référence la porte déjà — mais indispensable sur
/// une base d'avant l'Overframe. Renvoie `true` si la colonne a été ajoutée.
pub fn assurer_colonne_extended_art(conn: &Connection) -> Result<bool> {
    let existe = conn
        .prepare("SELECT 1 FROM pragma_table_info('set_prints') WHERE name = 'extended_art'")?
        .exists([])?;
    if existe {
        return Ok(false);
    }
    conn.execute(
        "ALTER TABLE set_prints ADD COLUMN extended_art INTEGER NOT NULL DEFAULT 0",
        [],
    )?;
    Ok(true)
}

/// Crée `overframe_sync` si elle manque.
///
/// Cette table ne fait pas partie du schéma d'initialisation : elle **survit**
/// aux reconstructions de la base (`TABLES_HORS_INIT`), justement pour que
/// l'idempotence par révision tienne d'une reconstruction à l'autre.
pub fn assurer_table_sync(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS overframe_sync (
            set_prefix TEXT PRIMARY KEY,
            revid      TEXT,
            synced_at  TEXT
        )",
        [],
    )?;
    Ok(())
}

/// Révision Yugipedia déjà appliquée pour ce préfixe, s'il y en a une.
pub fn revision_connue(conn: &Connection, prefixe: &str) -> Result<Option<String>> {
    assurer_table_sync(conn)?;
    let mut stmt = conn.prepare("SELECT revid FROM overframe_sync WHERE set_prefix = ?1")?;
    let mut lignes = stmt.query([prefixe])?;
    match lignes.next()? {
        Some(ligne) => Ok(ligne.get::<_, Option<String>>(0)?),
        None => Ok(None),
    }
}

/// Mémorise la révision appliquée.
///
/// `horodatage` est fourni par l'appelant plutôt que lu de l'horloge : c'est ce
/// qui rend la fonction reproductible en test.
pub fn enregistrer_revision(
    conn: &Connection,
    prefixe: &str,
    revid: &str,
    horodatage: &str,
) -> Result<()> {
    assurer_table_sync(conn)?;
    conn.execute(
        "INSERT INTO overframe_sync (set_prefix, revid, synced_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(set_prefix) DO UPDATE SET
             revid = excluded.revid, synced_at = excluded.synced_at",
        (prefixe, revid, horodatage),
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Réconciliation
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'a changé une réconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bilan {
    /// Lignes ajoutées en `extended_art = 1`.
    pub ajoutes: usize,
    /// Drapeaux corrigés sur des lignes existantes.
    pub corriges: usize,
}

/// Métadonnées héritables d'un numéro.
struct Meta {
    card_uuid: Option<String>,
    card_image_uuid: Option<String>,
    edition: String,
    qty: i64,
}

/// Applique le classement Yugipedia aux tirages d'un set déjà en base.
///
/// C'est le cœur de la passe, et il ne touche **ni au réseau ni à l'horloge** :
/// tout ce qu'il lui faut est la connexion et le classement. C'est ce qui rend
/// l'épreuve de bout en bout possible — rejouer les 282 tirages `LOCR-JP` de la
/// V1.0.3 et vérifier qu'on obtient les 318 de la V1.0.4, ligne pour ligne.
///
/// Renvoie `None` quand le set est absent de la base : il n'y a alors rien dont
/// hériter, et inventer des métadonnées serait pire que ne rien faire.
pub fn reconcilier(
    conn: &Connection,
    prefixe: &str,
    classement: &Classement,
) -> Result<Option<Bilan>> {
    assurer_colonne_extended_art(conn)?;

    // `ORDER BY id` ne change pas le résultat — `id` est l'alias du rowid, et
    // `LIKE` sur une colonne indexée en BINARY ne déclenche pas l'optimisation
    // d'index, donc SQLite balaie déjà la table dans cet ordre. La clause est
    // là pour que ça reste vrai quelle que soit la version de SQLite : la
    // première ligne rencontrée fixe le `set_uuid` et le `set_locale_id` de
    // toutes les lignes ajoutées.
    let mut stmt = conn.prepare(
        "SELECT set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code, rarity,
                edition, qty, print_image_url, COALESCE(extended_art, 0), id
         FROM set_prints WHERE set_code LIKE ?1 ORDER BY id",
    )?;
    let motif = format!("{prefixe}%");

    let mut premier: Option<(String, i64)> = None;
    let mut meta_par_numero: HashMap<String, Meta> = HashMap::new();
    let mut deja_extended: HashSet<(String, String)> = HashSet::new();
    let mut lignes_normales: HashMap<(String, String), i64> = HashMap::new();
    let mut url_par_numero_rarete: HashMap<(String, String), String> = HashMap::new();

    let mut lignes = stmt.query([&motif])?;
    while let Some(l) = lignes.next()? {
        let set_uuid: String = l.get(0)?;
        let locale_id: i64 = l.get(1)?;
        let card_uuid: Option<String> = l.get(2)?;
        let card_image_uuid: Option<String> = l.get(3)?;
        let set_code: String = l.get(4)?;
        let rarete: Option<String> = l.get(5)?;
        let edition: Option<String> = l.get(6)?;
        let qty: Option<i64> = l.get(7)?;
        let url: Option<String> = l.get(8)?;
        let extended: i64 = l.get(9)?;
        let id: i64 = l.get(10)?;

        if premier.is_none() {
            premier = Some((set_uuid, locale_id));
        }
        meta_par_numero.entry(set_code.clone()).or_insert(Meta {
            card_uuid,
            card_image_uuid,
            edition: edition.unwrap_or_else(|| "unlimited".to_owned()),
            qty: qty.unwrap_or(1),
        });

        // Une rareté NULL ne peut correspondre à aucun libellé Yugipedia : la
        // ligne n'entre dans aucun des index de comparaison. Le Python la range
        // sous une clé `None`, jamais consultée — même effet.
        let Some(rarete) = rarete else { continue };
        let cle = (set_code, rarete);
        if extended == 1 {
            deja_extended.insert(cle.clone());
        } else {
            lignes_normales.entry(cle.clone()).or_insert(id);
        }
        if let Some(url) = url {
            if !url.is_empty() {
                url_par_numero_rarete.entry(cle).or_insert(url);
            }
        }
    }
    drop(lignes);

    let Some((set_uuid, locale_id)) = premier else {
        return Ok(None);
    };

    // `classement.extended` est un ensemble ordonné : le parcours suit l'ordre
    // lexicographique, comme le `sorted(ext_set)` du Python. Ce n'est pas
    // cosmétique — il fixe l'ordre des insertions, donc les identifiants des
    // lignes créées.
    let mut a_inserer: Vec<(String, String)> = Vec::new();
    let mut a_corriger: Vec<i64> = Vec::new();
    for cle in &classement.extended {
        if deja_extended.contains(cle) {
            continue;
        }
        if classement.normal.contains(cle) {
            // La rareté existe dans les deux cadres : on ajoute la ligne
            // Overframe et on garde la normale.
            if meta_par_numero.contains_key(&cle.0) {
                a_inserer.push(cle.clone());
            }
        } else if let Some(&id) = lignes_normales.get(cle) {
            // Rareté Overframe seulement, que YGOJSON a importée en cadre
            // normal : on corrige le drapeau plutôt que de créer un doublon.
            a_corriger.push(id);
        } else if meta_par_numero.contains_key(&cle.0) {
            a_inserer.push(cle.clone());
        }
    }

    if !a_corriger.is_empty() {
        let mut maj = conn.prepare("UPDATE set_prints SET extended_art = 1 WHERE id = ?1")?;
        for id in &a_corriger {
            maj.execute([id])?;
        }
    }
    if !a_inserer.is_empty() {
        let mut ins = conn.prepare(
            "INSERT INTO set_prints
                (set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code, rarity,
                 edition, qty, print_image_url, extended_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)",
        )?;
        for (numero, rarete) in &a_inserer {
            let Some(meta) = meta_par_numero.get(numero) else {
                continue;
            };
            ins.execute(rusqlite::params![
                &set_uuid,
                locale_id,
                meta.card_uuid.as_deref().unwrap_or(""),
                meta.card_image_uuid.as_deref(),
                numero,
                rarete,
                &meta.edition,
                meta.qty,
                url_par_numero_rarete
                    .get(&(numero.clone(), rarete.clone()))
                    .map(String::as_str),
            ])?;
        }
    }

    Ok(Some(Bilan {
        ajoutes: a_inserer.len(),
        corriges: a_corriger.len(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Passe complète
// ─────────────────────────────────────────────────────────────────────────────

/// Issue d'une passe Overframe sur un set.
///
/// Le Python renvoie un dictionnaire dont la clé `status` porte une chaîne
/// parmi sept. Ici chaque cas est une variante, avec exactement les données qui
/// lui appartiennent : un `PageIntrouvable` n'a pas de compteur d'ajouts à lire
/// par accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// Réconciliation faite.
    Fait {
        /// Ce qui a changé.
        bilan: Bilan,
        /// Révision appliquée.
        revid: String,
    },
    /// Révision déjà appliquée — rien à faire.
    DejaAJour {
        /// Révision en base.
        revid: String,
    },
    /// Yugipedia ne déclare aucun tirage Overframe pour ce set.
    ///
    /// La révision est tout de même mémorisée : sans ça, chaque construction
    /// de base re-scraperait un set qui n'a rien à donner.
    AucunOverframe {
        /// Révision constatée.
        revid: String,
    },
    /// Aucune page « Set Card Lists » pour ce set.
    PageIntrouvable,
    /// Le set n'a aucun tirage dans `cardinfo.db`.
    AbsentCardinfo,
}

/// Complète les tirages Overframe d'un set.
///
/// Portage d'`enrich_set`. `force` ignore la garde d'idempotence — c'est ce que
/// déclenche la correction manuelle des Options, pour une base déjà marquée
/// synchronisée mais jamais réellement complétée.
///
/// # Différence avec le Python
///
/// `enrich_set` attrape toute exception et la renvoie en statut `"erreur"`,
/// de sorte qu'une panne réseau ressemble à un résultat. Ici l'échec remonte
/// en `Err` : c'est [`enrichir_whitelist`] qui décide de continuer, et il le
/// dit dans son rapport.
pub async fn enrichir_set(
    conn: &Connection,
    client: &ClientHttp,
    set: &SetOverframe,
    force: bool,
    horodatage: &str,
) -> Result<Issue> {
    assurer_colonne_extended_art(conn)?;

    let pages = yugipedia::resoudre_pages(client, set.nom_anglais, Some(set.region)).await?;
    let Some(titre) = pages.first() else {
        return Ok(Issue::PageIntrouvable);
    };

    let (entrees, revid) = yugipedia::lire_set_list(client, titre).await?;
    let revid = revid.map(|r| r.to_string()).unwrap_or_default();

    if !force && revision_connue(conn, set.prefixe)?.as_deref() == Some(revid.as_str()) {
        return Ok(Issue::DejaAJour { revid });
    }

    let classement = classer(&entrees);
    if classement.extended.is_empty() {
        enregistrer_revision(conn, set.prefixe, &revid, horodatage)?;
        return Ok(Issue::AucunOverframe { revid });
    }

    match reconcilier(conn, set.prefixe, &classement)? {
        None => Ok(Issue::AbsentCardinfo),
        Some(bilan) => {
            enregistrer_revision(conn, set.prefixe, &revid, horodatage)?;
            tracing::info!(
                set = set.prefixe,
                ajoutes = bilan.ajoutes,
                corriges = bilan.corriges,
                revid = %revid,
                "Overframe : set complété"
            );
            Ok(Issue::Fait { bilan, revid })
        }
    }
}

/// Résultat d'une passe sur un set, échec compris.
#[derive(Debug)]
pub struct Rapport {
    /// Préfixe du set.
    pub prefixe: &'static str,
    /// Issue, ou l'erreur qui a interrompu ce set seulement.
    pub issue: std::result::Result<Issue, AppError>,
}

/// Passe Overframe sur tous les sets connus.
///
/// Portage d'`enrich_whitelist`. Un set en échec **n'arrête pas** les autres :
/// c'est la seule façon qu'une coupure réseau au deuxième set ne fasse pas
/// perdre le premier.
pub async fn enrichir_whitelist(
    conn: &Connection,
    client: &ClientHttp,
    force: bool,
    horodatage: &str,
) -> Vec<Rapport> {
    let mut rapports = Vec::with_capacity(SETS_OVERFRAME.len());
    for set in &SETS_OVERFRAME {
        let issue = enrichir_set(conn, client, set, force, horodatage).await;
        if let Err(e) = &issue {
            tracing::warn!(set = set.prefixe, erreur = %e, "Overframe : set ignoré");
        }
        rapports.push(Rapport {
            prefixe: set.prefixe,
            issue,
        });
    }
    rapports
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn entree(numero: &str, raretes: &[&str], note: &str) -> EntreeSetList {
        EntreeSetList {
            numero: numero.to_owned(),
            nom: "Carte".to_owned(),
            raretes: raretes.iter().map(|r| (*r).to_owned()).collect(),
            extended_art: note.to_lowercase().contains("extended art"),
            note: note.to_owned(),
        }
    }

    fn cle(numero: &str, rarete: &str) -> (String, String) {
        (numero.to_owned(), rarete.to_owned())
    }

    // ── Classification ──────────────────────────────────────────────────────

    #[test]
    fn la_note_extended_art_classe_l_entree() {
        let e = entree("A-JP001", &["Ultra Rare"], "description::(extended art)");
        assert!(entree_est_overframe(&e, &["Ultra Rare".to_owned()]));
    }

    #[test]
    fn les_autres_formulations_de_la_note_comptent_aussi() {
        for note in [
            "(Overframe)",
            "over frame",
            "EXTENDED ARTWORK",
            "over flame",
        ] {
            let e = entree("A-JP001", &["Rare"], note);
            assert!(
                entree_est_overframe(&e, &["Rare".to_owned()]),
                "note « {note} » non reconnue"
            );
        }
    }

    #[test]
    fn la_grand_master_rare_classe_l_entree_sans_aucune_note() {
        // Le signal le plus fiable : cette rareté n'existe pas en cadre normal.
        let e = entree("A-JP001", &["Grand Master Rare"], "");
        assert!(!e.extended_art);
        assert!(entree_est_overframe(&e, &["Grand Master Rare".to_owned()]));
        // Et sa variante sans espace.
        let e = entree("A-JP001", &["GrandMaster Rare"], "");
        assert!(entree_est_overframe(&e, &["GrandMaster Rare".to_owned()]));
    }

    #[test]
    fn une_entree_overframe_entraine_toutes_ses_raretes() {
        let entrees = vec![entree(
            "A-JP001",
            &["Ultra Rare", "Prismatic Secret Rare", "Grand Master Rare"],
            "",
        )];
        let c = classer(&entrees);
        assert_eq!(c.extended.len(), 3);
        assert!(c.normal.is_empty());
    }

    #[test]
    fn une_rarete_peut_etre_dans_les_deux_cadres() {
        let entrees = vec![
            entree("A-JP001", &["Ultra Rare", "Secret Rare"], ""),
            entree(
                "A-JP001",
                &["Ultra Rare", "Prismatic Secret Rare", "Grand Master Rare"],
                "description::(extended art)",
            ),
        ];
        let c = classer(&entrees);
        assert!(c.normal.contains(&cle("A-JP001", "Ultra Rare")));
        assert!(c.extended.contains(&cle("A-JP001", "Ultra Rare")));
        assert!(!c.normal.contains(&cle("A-JP001", "Grand Master Rare")));
    }

    #[test]
    fn une_entree_sans_rarete_est_ignoree() {
        let entrees = vec![entree("A-JP001", &[], "extended art")];
        assert_eq!(classer(&entrees), Classement::default());
    }

    // ── Réconciliation ──────────────────────────────────────────────────────

    /// Base minimale : `set_prints` seule, sans les contraintes de clé
    /// étrangère — la réconciliation n'y touche pas.
    fn base_avec(lignes: &[(&str, &str, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                set_uuid        TEXT NOT NULL,
                set_locale_id   INTEGER NOT NULL,
                card_uuid       TEXT NOT NULL,
                card_image_uuid TEXT,
                set_code        TEXT,
                rarity          TEXT,
                edition         TEXT,
                qty             INTEGER DEFAULT 1,
                print_image_url TEXT,
                extended_art    INTEGER NOT NULL DEFAULT 0)",
        )
        .unwrap();
        for (code, rarete, ext) in lignes {
            conn.execute(
                "INSERT INTO set_prints
                    (set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                     rarity, edition, qty, print_image_url, extended_art)
                 VALUES ('SU', 7, 'CU', 'CIU', ?1, ?2, '1st', 3, 'http://img', ?3)",
                rusqlite::params![code, rarete, ext],
            )
            .unwrap();
        }
        conn
    }

    fn contenu(conn: &Connection) -> Vec<(String, String, i64)> {
        let mut stmt = conn
            .prepare("SELECT set_code, rarity, extended_art FROM set_prints ORDER BY id")
            .unwrap();
        let v: Vec<_> = stmt
            .query_map([], |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?)))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        v
    }

    #[test]
    fn une_rarete_des_deux_cadres_donne_une_ligne_de_plus() {
        let conn = base_avec(&[("A-JP001", "Prismatic Secret Rare", 0)]);
        let mut c = Classement::default();
        c.normal.insert(cle("A-JP001", "Prismatic Secret Rare"));
        c.extended.insert(cle("A-JP001", "Prismatic Secret Rare"));
        let bilan = reconcilier(&conn, "A-JP", &c).unwrap().unwrap();
        assert_eq!(
            bilan,
            Bilan {
                ajoutes: 1,
                corriges: 0
            }
        );
        assert_eq!(
            contenu(&conn),
            [
                ("A-JP001".into(), "Prismatic Secret Rare".into(), 0),
                ("A-JP001".into(), "Prismatic Secret Rare".into(), 1),
            ]
        );
    }

    #[test]
    fn une_rarete_overframe_seule_corrige_le_drapeau_sans_doublon() {
        let conn = base_avec(&[("A-JP001", "Grand Master Rare", 0)]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        let bilan = reconcilier(&conn, "A-JP", &c).unwrap().unwrap();
        assert_eq!(
            bilan,
            Bilan {
                ajoutes: 0,
                corriges: 1
            }
        );
        assert_eq!(
            contenu(&conn),
            [("A-JP001".into(), "Grand Master Rare".into(), 1)]
        );
    }

    #[test]
    fn une_ligne_deja_overframe_n_est_pas_retouchee() {
        let conn = base_avec(&[("A-JP001", "Grand Master Rare", 1)]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        assert_eq!(
            reconcilier(&conn, "A-JP", &c).unwrap().unwrap(),
            Bilan::default()
        );
        assert_eq!(contenu(&conn).len(), 1);
    }

    #[test]
    fn la_passe_est_idempotente() {
        let conn = base_avec(&[
            ("A-JP001", "Prismatic Secret Rare", 0),
            ("A-JP001", "Grand Master Rare", 0),
        ]);
        let mut c = Classement::default();
        c.normal.insert(cle("A-JP001", "Prismatic Secret Rare"));
        c.extended.insert(cle("A-JP001", "Prismatic Secret Rare"));
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));

        let premier = reconcilier(&conn, "A-JP", &c).unwrap().unwrap();
        assert_eq!(
            premier,
            Bilan {
                ajoutes: 1,
                corriges: 1
            }
        );
        let apres = contenu(&conn);
        // Rejouer la même réconciliation ne doit plus rien changer.
        assert_eq!(
            reconcilier(&conn, "A-JP", &c).unwrap().unwrap(),
            Bilan::default()
        );
        assert_eq!(contenu(&conn), apres);
    }

    #[test]
    fn rien_n_est_jamais_supprime() {
        // Une rareté en base qu'aucune Set list ne mentionne survit.
        let conn = base_avec(&[
            ("A-JP001", "Rareté oubliée", 0),
            ("A-JP001", "Grand Master Rare", 0),
        ]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        reconcilier(&conn, "A-JP", &c).unwrap();
        assert!(contenu(&conn).iter().any(|(_, r, _)| r == "Rareté oubliée"));
    }

    #[test]
    fn un_numero_absent_de_la_base_n_invente_pas_de_metadonnees() {
        let conn = base_avec(&[("A-JP001", "Common", 0)]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP999", "Grand Master Rare"));
        assert_eq!(
            reconcilier(&conn, "A-JP", &c).unwrap().unwrap(),
            Bilan::default()
        );
        assert_eq!(contenu(&conn).len(), 1);
    }

    #[test]
    fn un_set_absent_de_la_base_ne_donne_aucun_bilan() {
        let conn = base_avec(&[("B-JP001", "Common", 0)]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        assert!(reconcilier(&conn, "A-JP", &c).unwrap().is_none());
    }

    #[test]
    fn les_metadonnees_sont_heritees_du_numero() {
        let conn = base_avec(&[("A-JP001", "Common", 0)]);
        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        reconcilier(&conn, "A-JP", &c).unwrap();
        let (uuid, locale, cu, ciu, ed, qty): (String, i64, String, Option<String>, String, i64) =
            conn.query_row(
                "SELECT set_uuid, set_locale_id, card_uuid, card_image_uuid, edition, qty
                 FROM set_prints WHERE extended_art = 1",
                [],
                |l| {
                    Ok((
                        l.get(0)?,
                        l.get(1)?,
                        l.get(2)?,
                        l.get(3)?,
                        l.get(4)?,
                        l.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!((uuid.as_str(), locale, cu.as_str()), ("SU", 7, "CU"));
        assert_eq!(ciu.as_deref(), Some("CIU"));
        assert_eq!((ed.as_str(), qty), ("1st", 3));
    }

    #[test]
    fn les_metadonnees_absentes_prennent_les_defauts_du_python() {
        // `ed if ed is not None else "unlimited"` et `q if q is not None else 1`.
        // Aucune ligne des bases réelles n'exerce ce repli — d'où ce test à la
        // main, sans quoi la branche ne serait couverte par rien.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                set_uuid        TEXT NOT NULL,
                set_locale_id   INTEGER NOT NULL,
                card_uuid       TEXT NOT NULL,
                card_image_uuid TEXT,
                set_code        TEXT,
                rarity          TEXT,
                edition         TEXT,
                qty             INTEGER,
                print_image_url TEXT,
                extended_art    INTEGER NOT NULL DEFAULT 0);
             INSERT INTO set_prints
                (set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                 rarity, edition, qty, print_image_url, extended_art)
             VALUES ('SU', 7, 'CU', NULL, 'A-JP001', 'Common', NULL, NULL, NULL, 0)",
        )
        .unwrap();

        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        reconcilier(&conn, "A-JP", &c).unwrap();

        let (edition, qty, ciu, url): (String, i64, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT edition, qty, card_image_uuid, print_image_url
                 FROM set_prints WHERE extended_art = 1",
                [],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)),
            )
            .unwrap();
        assert_eq!(edition, "unlimited");
        assert_eq!(qty, 1);
        // Ce qui est nul le reste : on n'invente ni illustration ni URL.
        assert_eq!(ciu, None);
        assert_eq!(url, None);
    }

    #[test]
    fn un_card_uuid_nul_devient_une_chaine_vide() {
        // La colonne est `NOT NULL` : le Python écrit `m["card_uuid"] or ""`.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                set_uuid        TEXT NOT NULL,
                set_locale_id   INTEGER NOT NULL,
                card_uuid       TEXT,
                card_image_uuid TEXT,
                set_code        TEXT,
                rarity          TEXT,
                edition         TEXT,
                qty             INTEGER,
                print_image_url TEXT,
                extended_art    INTEGER NOT NULL DEFAULT 0);
             INSERT INTO set_prints
                (set_uuid, set_locale_id, card_uuid, set_code, rarity, extended_art)
             VALUES ('SU', 7, NULL, 'A-JP001', 'Common', 0)",
        )
        .unwrap();

        let mut c = Classement::default();
        c.extended.insert(cle("A-JP001", "Grand Master Rare"));
        reconcilier(&conn, "A-JP", &c).unwrap();

        let cu: String = conn
            .query_row(
                "SELECT card_uuid FROM set_prints WHERE extended_art = 1",
                [],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(cu, "");
    }

    #[test]
    fn les_insertions_suivent_l_ordre_lexicographique() {
        // L'ordre fixe les identifiants créés : il fait partie du résultat.
        let conn = base_avec(&[("A-JP002", "Common", 0), ("A-JP001", "Common", 0)]);
        let mut c = Classement::default();
        for (n, r) in [
            ("A-JP002", "Ultra Rare"),
            ("A-JP001", "Ultra Rare"),
            ("A-JP001", "Grand Master Rare"),
        ] {
            c.extended.insert(cle(n, r));
        }
        reconcilier(&conn, "A-JP", &c).unwrap();
        let ajoutees: Vec<_> = contenu(&conn)
            .into_iter()
            .filter(|(_, _, e)| *e == 1)
            .map(|(n, r, _)| format!("{n} {r}"))
            .collect();
        assert_eq!(
            ajoutees,
            [
                "A-JP001 Grand Master Rare",
                "A-JP001 Ultra Rare",
                "A-JP002 Ultra Rare"
            ]
        );
    }

    // ── Schéma ──────────────────────────────────────────────────────────────

    #[test]
    fn la_colonne_extended_art_est_ajoutee_une_seule_fois() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE set_prints (id INTEGER PRIMARY KEY, set_code TEXT)")
            .unwrap();
        assert!(assurer_colonne_extended_art(&conn).unwrap());
        assert!(!assurer_colonne_extended_art(&conn).unwrap());
    }

    #[test]
    fn la_revision_se_memorise_et_s_ecrase() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(revision_connue(&conn, "LOCR-JP").unwrap(), None);
        enregistrer_revision(&conn, "LOCR-JP", "5893931", "2026-06-01 00:47:25").unwrap();
        assert_eq!(
            revision_connue(&conn, "LOCR-JP").unwrap().as_deref(),
            Some("5893931")
        );
        enregistrer_revision(&conn, "LOCR-JP", "5945579", "2026-08-25 21:30:14").unwrap();
        assert_eq!(
            revision_connue(&conn, "LOCR-JP").unwrap().as_deref(),
            Some("5945579")
        );
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM overframe_sync", [], |l| l.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn la_whitelist_porte_les_deux_sets_du_python() {
        assert_eq!(SETS_OVERFRAME.len(), 2);
        assert_eq!(SETS_OVERFRAME[0].prefixe, "LOCH-JP");
        assert_eq!(SETS_OVERFRAME[1].prefixe, "LOCR-JP");
    }
}
