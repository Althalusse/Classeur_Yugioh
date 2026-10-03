// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Création d'un classeur — construction des lignes depuis `cardinfo.db`.
//!
//! Portage du chemin **local** de
//! `module/creation_classeur/creation_classeur_service.py` (2 134 lignes) :
//! `_build_rows_from_local_db`, son tri, et l'heuristique qui décide si le
//! résultat mérite d'être remplacé par un appel à YGOPRODeck.
//!
//! # Pourquoi ce chemin d'abord
//!
//! L'application tente **toujours** la construction locale en premier : elle ne
//! coûte aucune requête et donne la structure physique réelle du set. L'appel
//! d'API n'est qu'un repli, déclenché par l'heuristique de
//! [`locales_semblent_incompletes`].
//!
//! # Le choix de la locale d'énumération
//!
//! C'est la décision la plus délicate du module, et elle a coûté un bug de
//! production. Énumérer depuis la locale française évite les doublons
//! d'artworks historiques que YGOJSON traîne sur les cartes-icônes. Mais sur un
//! set récent, YGOJSON peut n'avoir importé la locale française que
//! **partiellement** — une rareté sur sept, parce que Yugipedia n'a pas encore
//! publié les traductions.
//!
//! Une condition « FR non vide » prenait alors la française et le classeur
//! perdait six raretés sur sept. C'est exactement ce qui est arrivé à `RA05` en
//! avril 2026 : une rareté créée au lieu de sept, tandis que `RA02` — set
//! ancien à traduction complète — fonctionnait. D'où la règle actuelle : la
//! locale française n'est retenue que si elle est **au moins aussi complète**
//! que l'anglaise.
//!
//! Un classeur OCG japonais, lui, force sa locale sans repli : mélanger des
//! tirages français ou anglais dans un classeur japonais produirait des codes
//! incohérents avec ce que le collectionneur a en main.
//!
//! # Ce que le tri décide
//!
//! Le rang séquentiel produit ici **est la place de la carte dans la grille**.
//! La clé de tri est celle de [`ygo_core::tri::cle_numero`] — le même
//! algorithme que l'affichage, sans quoi la création et la consultation
//! ordonneraient les cartes différemment.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;
use ygo_core::config::a_suffixe_ocg;
use ygo_core::paths::Paths;
use ygo_core::tri::cle_numero;

use crate::error::{AppError, Result};

/// Moyenne de raretés par carte en dessous de laquelle les données locales
/// sont jugées incomplètes (`SEUIL_AVG_RARETES_SUSPECT`).
///
/// # Pourquoi 2,0
///
/// Les boosters et les Rarity Collection ont au moins deux raretés par carte,
/// souvent trois à sept. Les Structure Decks n'en ont qu'une — le repli sur
/// YGOPRODeck se déclenche donc à tort pour eux, mais sans dommage : l'API
/// rendra elle aussi une rareté par carte. Coût : un appel inutile sur des sets
/// rarement créés. Pour `RA05` (moyenne 1,52), le seuil déclenche bien.
pub const SEUIL_AVG_RARETES_SUSPECT: f64 = 2.0;

/// Une ligne de la table `cards` d'un classeur, avant écriture.
///
/// Reprend les clés du dictionnaire Python, dans l'ordre des colonnes du DDL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LigneClasseur {
    /// UUID de la carte dans `cardinfo.db`.
    pub card_uuid: String,
    /// UUID de l'illustration.
    pub card_image_uuid: String,
    /// Identifiant d'image — négatif pour un artwork externe.
    pub card_image_id: Option<i64>,
    /// Nom anglais.
    pub name: String,
    /// Nom français, vide s'il n'est pas connu.
    pub name_fr: String,
    /// Code de set, **anglais** quand il a pu être retrouvé.
    pub set_code: String,
    /// Libellé de rareté.
    pub rarity: String,
    /// Code de rareté — toujours vide sur ce chemin, la colonne étant
    /// propre à YGOPRODeck.
    pub rarity_code: String,
    /// Nom anglais du set.
    pub set_name: String,
    /// URL de la carte entière.
    pub card_image_url: String,
    /// URL de l'artwork seul.
    pub card_image_small: String,
    /// Rang séquentiel — **la place dans la grille**.
    pub sort_order: i64,
    /// Type de carte.
    pub card_type: String,
    /// Attaque.
    pub atk: Option<i64>,
    /// Défense.
    pub def_val: Option<i64>,
    /// Niveau / rang.
    pub level: Option<i64>,
    /// Attribut — **nullable**.
    ///
    /// YGOPRODeck rend `null` pour les magies et les pièges, qui n'en ont pas,
    /// et le Python laisse passer ce `null` jusqu'à la base. Le chemin local,
    /// lui, écrit toujours une chaîne (`attribute or ""`). Les deux chemins
    /// diffèrent donc sur ce champ, et le portage reproduit les deux : 200
    /// cartes sur les 507 capturées sont concernées.
    pub attribute: Option<String>,
    /// Type (Dragon, Spellcaster…).
    pub race: String,
    /// 1 si Overframe.
    pub extended_art: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tri
// ─────────────────────────────────────────────────────────────────────────────

/// Numéro ordinal lu à la fin d'un code de set.
///
/// Portage de `_sort_order_from_code` : `SS01-ENA01` → 1, `RA05-EN035` → 35.
/// Ne sert que de valeur initiale à `sort_order`, **de toute façon écrasée**
/// par le rang après le tri. Conservé parce que le chemin API s'en sert.
///
/// À ne pas confondre avec la clé de tri : celle-ci exige que tout ce qui suit
/// le préfixe de langue soit lettres puis chiffres, là où celle-là se contente
/// des chiffres de fin.
pub fn ordre_depuis_code(set_code: &str) -> i64 {
    let suffixe = match set_code.rfind('-') {
        Some(i) => set_code.get(i + 1..).unwrap_or(""),
        None => set_code,
    };
    let chiffres: String = suffixe
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    chiffres.parse().unwrap_or(0)
}

/// Clé de tri d'un code de set : groupe de lettres, puis numéro.
///
/// C'est **exactement** [`ygo_core::tri::cle_numero`], et ce n'est pas une
/// commodité : le Python le dit dans sa docstring — « doit rester aligné avec
/// `tri_carte._extract_numero` (même algorithme) ». Si les deux divergeaient,
/// la création rangerait les cartes dans un ordre et l'affichage dans un autre.
/// Réutiliser la même fonction rend la divergence impossible plutôt
/// qu'improbable.
pub fn cle_tri(set_code: &str) -> (&str, i64) {
    cle_numero(set_code)
}

/// Trie les lignes puis les renumérote de `0` à `n-1`.
///
/// Portage du tri final de `_build_rows_from_local_db`. Trois niveaux :
///
/// 1. la clé de numéro — qui groupe les sous-decks d'un set multi-deck ;
/// 2. le cadre — **l'Overframe forme un bloc distinct**, placé après ;
/// 3. le libellé de rareté, comparé tel quel.
///
/// Le tri est stable, comme celui de Python : deux lignes de clé identique
/// gardent leur ordre d'arrivée, c'est-à-dire celui de la requête SQL.
pub fn trier_et_numeroter(lignes: &mut [LigneClasseur]) {
    lignes.sort_by(|a, b| {
        cle_tri(&a.set_code)
            .cmp(&cle_tri(&b.set_code))
            .then_with(|| bloc_cadre(a).cmp(&bloc_cadre(b)))
            .then_with(|| a.rarity.cmp(&b.rarity))
    });
    for (rang, ligne) in lignes.iter_mut().enumerate() {
        ligne.sort_order = rang as i64;
    }
}

/// `1` pour l'Overframe, `0` sinon — le Python teste la valeur, pas l'égalité
/// à 1, donc toute valeur non nulle vaut Overframe.
fn bloc_cadre(ligne: &LigneClasseur) -> i64 {
    i64::from(ligne.extended_art != 0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Heuristique d'incomplétude
// ─────────────────────────────────────────────────────────────────────────────

/// Verdict de l'heuristique d'incomplétude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Incompletude {
    /// Les données locales sont-elles jugées insuffisantes ?
    pub suspect: bool,
    /// Moyenne de raretés par carte.
    pub moyenne: f64,
    /// Nombre de cartes distinctes.
    pub cartes_uniques: usize,
}

/// Les lignes locales semblent-elles incomplètes ?
///
/// Portage de `_rows_locales_semblent_incompletes`. Diagnostic confirmé sur
/// `RA05` en mai 2026 : YGOJSON rendait 228 tirages pour 150 cartes — moyenne
/// 1,52 — quand YGOPRODeck en donne sept par carte du main pool.
///
/// Une liste sans aucune carte identifiée n'est **pas** jugée suspecte : sans
/// dénominateur, la moyenne n'a pas de sens, et le Python préfère ne rien
/// conclure.
pub fn locales_semblent_incompletes(lignes: &[LigneClasseur]) -> Incompletude {
    let uniques: std::collections::HashSet<&str> = lignes
        .iter()
        .map(|l| l.card_uuid.as_str())
        .filter(|u| !u.is_empty())
        .collect();
    if uniques.is_empty() {
        return Incompletude {
            suspect: false,
            moyenne: 0.0,
            cartes_uniques: 0,
        };
    }
    let moyenne = lignes.len() as f64 / uniques.len() as f64;
    Incompletude {
        suspect: moyenne < SEUIL_AVG_RARETES_SUSPECT,
        moyenne,
        cartes_uniques: uniques.len(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Choix de la locale
// ─────────────────────────────────────────────────────────────────────────────

/// Locale retenue pour énumérer les tirages d'un set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeEnumeration {
    /// Locale française — évite les doublons d'artworks historiques.
    Francaise,
    /// Locale anglaise.
    Anglaise,
    /// Locale japonaise — imposée pour un classeur OCG, sans repli.
    Japonaise,
}

/// Ce que le choix de locale a décidé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChoixLocale {
    /// Locale d'énumération.
    pub mode: ModeEnumeration,
    /// Identifiant de la locale énumérée.
    pub locale_enumeree: i64,
    /// Identifiant de la locale anglaise, pour retrouver le code de set
    /// anglais quand on énumère en français.
    pub locale_anglaise: Option<i64>,
}

/// Choisit la locale d'énumération d'un set.
///
/// Portage de l'étape 2 de `_build_rows_from_local_db`. Voir l'en-tête du
/// module pour la raison d'être de la comparaison de complétude.
pub fn choisir_locale(conn: &Connection, code_set: &str, set_uuid: &str) -> Result<ChoixLocale> {
    if a_suffixe_ocg(code_set) {
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM set_locales WHERE set_uuid = ?1 AND language = 'jp' LIMIT 1",
                [set_uuid],
                |l| l.get(0),
            )
            .ok();
        let Some(id) = id else {
            return Err(AppError::SetAbsentDeLaBase(format!(
                "set OCG « {code_set} » présent, mais aucune locale japonaise"
            )));
        };
        return Ok(ChoixLocale {
            mode: ModeEnumeration::Japonaise,
            locale_enumeree: id,
            // Pas de jointure anglaise en mode japonais : les codes de set
            // doivent rester ceux de la locale énumérée.
            locale_anglaise: None,
        });
    }

    let mut stmt = conn.prepare(
        "SELECT language, id FROM set_locales
         WHERE set_uuid = ?1 AND language IN ('fr', 'en')",
    )?;
    let mut ids: HashMap<String, i64> = HashMap::new();
    let mut lignes = stmt.query([set_uuid])?;
    while let Some(l) = lignes.next()? {
        ids.insert(l.get::<_, String>(0)?, l.get::<_, i64>(1)?);
    }
    drop(lignes);

    let compter = |id: i64| -> Result<i64> {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM set_prints WHERE set_locale_id = ?1",
            [id],
            |l| l.get(0),
        )?)
    };
    let fr = ids.get("fr").copied();
    let en = ids.get("en").copied();
    let n_fr = match fr {
        Some(id) => compter(id)?,
        None => 0,
    };
    let n_en = match en {
        Some(id) => compter(id)?,
        None => 0,
    };

    // La française n'est retenue que si elle est au moins aussi fournie.
    if n_fr > 0 && n_fr >= n_en {
        if let Some(id) = fr {
            return Ok(ChoixLocale {
                mode: ModeEnumeration::Francaise,
                locale_enumeree: id,
                locale_anglaise: en,
            });
        }
    }
    if n_en > 0 {
        if let Some(id) = en {
            return Ok(ChoixLocale {
                mode: ModeEnumeration::Anglaise,
                locale_enumeree: id,
                locale_anglaise: Some(id),
            });
        }
    }
    Err(AppError::SetAbsentDeLaBase(format!(
        "set « {code_set} » présent, mais aucune locale française ni anglaise pourvue"
    )))
}

// ─────────────────────────────────────────────────────────────────────────────
// Construction
// ─────────────────────────────────────────────────────────────────────────────

/// Retrouve le set et son nom anglais depuis un préfixe de classeur.
///
/// Un classeur OCG porte un préfixe **complet** (`LOCR-JP`) qui correspond
/// directement à `prefix` ; un classeur TCG porte un préfixe nu (`RA05`) qui
/// doit accepter `RA05-EN`, `RA05-EU`… d'où les deux conditions.
fn resoudre_set(conn: &Connection, code_set: &str) -> Result<(String, String)> {
    let resultat = conn.query_row(
        "SELECT DISTINCT s.uuid, s.name_en
         FROM sets s JOIN set_locales sl ON sl.set_uuid = s.uuid
         WHERE sl.prefix = ?1 OR sl.prefix LIKE ?1 || '-%'
         LIMIT 1",
        [code_set],
        |l| Ok((l.get::<_, String>(0)?, l.get::<_, Option<String>>(1)?)),
    );
    match resultat {
        Ok((uuid, nom)) => Ok((uuid, nom.unwrap_or_default())),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(AppError::SetAbsentDeLaBase(format!(
            "set « {code_set} » absent de cardinfo.db"
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Colonnes communes aux deux requêtes d'énumération.
const COLONNES: &str = "
    ?SET_CODE?                 AS set_code,
    ?RARITY?                   AS rarity,
    ci.ygoprodeck_image_id     AS card_image_id,
    ci.card_url                AS card_image_url,
    ci.art_url                 AS card_image_small,
    c.uuid                     AS card_uuid,
    ci.uuid                    AS card_image_uuid,
    c.card_type                AS card_type,
    c.atk                      AS atk,
    c.def                      AS def_val,
    c.level                    AS level,
    c.attribute                AS attribute,
    c.race                     AS race,
    ct_en.name                 AS name_en,
    ct_fr.name                 AS name_fr,
    ?EXTENDED?                 AS extended_art";

/// Construit les lignes d'un classeur depuis `cardinfo.db` seule.
///
/// Portage de `_build_rows_from_local_db`. Aucune requête réseau : c'est tout
/// l'intérêt de ce chemin.
///
/// # La jointure anglaise, et pourquoi elle ne contraint plus la rareté
///
/// Quand on énumère en français, il faut retrouver le code de set anglais. La
/// jointure exigeait à l'origine une rareté identique des deux côtés. Trop
/// strict : plusieurs reprints de Structure Deck portent `Common` en français
/// et `Short Print` en anglais. La jointure échouait, le code français
/// subsistait (`SDWD-FR013`), et l'import CSV — qui convertit toujours vers
/// l'anglais — ne retrouvait plus ces cartes. Six cartes perdues sur `SDWD`.
///
/// Or pour une carte physique donnée, le code de set **ne dépend pas de la
/// rareté** : le numéro de collection reste le même qu'elle sorte en Common ou
/// en Secret Rare. La contrainte n'apportait rien et cassait le rattrapage.
/// Elle a donc sauté, avec un `MIN(set_code)` pour rester déterministe si
/// plusieurs codes anglais coexistaient.
pub fn construire_lignes_locales(conn: &Connection, code_set: &str) -> Result<Vec<LigneClasseur>> {
    let (set_uuid, set_name_en) = resoudre_set(conn, code_set)?;
    let choix = choisir_locale(conn, code_set, &set_uuid)?;

    let mut lignes = match (choix.mode, choix.locale_anglaise) {
        (ModeEnumeration::Francaise, Some(locale_en)) => {
            let sql = format!(
                "SELECT {}
                 FROM set_prints sp_fr
                 LEFT JOIN (
                     SELECT card_uuid, card_image_uuid, MIN(set_code) AS set_code_min
                     FROM set_prints WHERE set_locale_id = ?1
                     GROUP BY card_uuid, card_image_uuid
                 ) sp_en
                     ON sp_en.card_uuid = sp_fr.card_uuid
                    AND sp_en.card_image_uuid = sp_fr.card_image_uuid
                 JOIN cards c ON c.uuid = sp_fr.card_uuid
                 LEFT JOIN card_images ci ON ci.uuid = sp_fr.card_image_uuid
                 LEFT JOIN card_texts ct_en ON ct_en.card_uuid = c.uuid AND ct_en.language = 'en'
                 LEFT JOIN card_texts ct_fr ON ct_fr.card_uuid = c.uuid AND ct_fr.language = 'fr'
                 WHERE sp_fr.set_locale_id = ?2
                 ORDER BY COALESCE(sp_en.set_code_min, sp_fr.set_code), sp_fr.rarity",
                COLONNES
                    .replace("?SET_CODE?", "COALESCE(sp_en.set_code_min, sp_fr.set_code)")
                    .replace("?RARITY?", "sp_fr.rarity")
                    .replace("?EXTENDED?", "sp_fr.extended_art")
            );
            lire(
                conn,
                &sql,
                &[&locale_en, &choix.locale_enumeree],
                &set_name_en,
            )?
        }
        _ => {
            let sql = format!(
                "SELECT {}
                 FROM set_prints sp
                 JOIN cards c ON c.uuid = sp.card_uuid
                 LEFT JOIN card_images ci ON ci.uuid = sp.card_image_uuid
                 LEFT JOIN card_texts ct_en ON ct_en.card_uuid = c.uuid AND ct_en.language = 'en'
                 LEFT JOIN card_texts ct_fr ON ct_fr.card_uuid = c.uuid AND ct_fr.language = 'fr'
                 WHERE sp.set_locale_id = ?1
                 ORDER BY sp.set_code, sp.rarity",
                COLONNES
                    .replace("?SET_CODE?", "sp.set_code")
                    .replace("?RARITY?", "sp.rarity")
                    .replace("?EXTENDED?", "sp.extended_art")
            );
            lire(conn, &sql, &[&choix.locale_enumeree], &set_name_en)?
        }
    };

    if lignes.is_empty() {
        return Err(AppError::SetAbsentDeLaBase(format!(
            "set « {code_set} » connu, mais aucune carte trouvée"
        )));
    }
    trier_et_numeroter(&mut lignes);
    Ok(lignes)
}

fn lire(
    conn: &Connection,
    sql: &str,
    parametres: &[&dyn rusqlite::ToSql],
    set_name_en: &str,
) -> Result<Vec<LigneClasseur>> {
    let mut stmt = conn.prepare(sql)?;
    let mut lignes = Vec::new();
    let mut rangs = stmt.query(parametres)?;
    while let Some(l) = rangs.next()? {
        let texte = |i: usize| -> rusqlite::Result<String> {
            Ok(l.get::<_, Option<String>>(i)?.unwrap_or_default())
        };
        lignes.push(LigneClasseur {
            set_code: texte(0)?,
            rarity: texte(1)?,
            card_image_id: l.get(2)?,
            card_image_url: texte(3)?,
            card_image_small: texte(4)?,
            card_uuid: texte(5)?,
            card_image_uuid: texte(6)?,
            card_type: texte(7)?,
            atk: l.get(8)?,
            def_val: l.get(9)?,
            level: l.get(10)?,
            attribute: Some(texte(11)?),
            race: texte(12)?,
            name: texte(13)?,
            name_fr: texte(14)?,
            extended_art: l.get::<_, Option<i64>>(15)?.unwrap_or(0),
            // Vide sur ce chemin : la colonne appartient à YGOPRODeck.
            rarity_code: String::new(),
            set_name: set_name_en.to_owned(),
            // Écrasé par le rang, juste après.
            sort_order: 0,
        });
    }
    Ok(lignes)
}

// ─────────────────────────────────────────────────────────────────────────────
// Repli YGOPRODeck
// ─────────────────────────────────────────────────────────────────────────────

/// Un set tel que `cardsets.php` le décrit.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SetYgoprodeck {
    /// Préfixe du set, `RA05`.
    #[serde(default)]
    pub set_code: String,
    /// Nom complet, seul identifiant accepté par `cardinfo.php`.
    #[serde(default)]
    pub set_name: String,
}

/// Une carte telle que `cardinfo.php` la rend, réduite à ce qui sert.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CarteApi {
    /// Identifiant YGOPRODeck — le « password » Konami.
    #[serde(default)]
    pub id: Option<i64>,
    /// Nom, dans la langue demandée.
    #[serde(default)]
    pub name: String,
    /// Type de carte.
    #[serde(rename = "type", default)]
    pub type_carte: String,
    /// Attaque.
    #[serde(default)]
    pub atk: Option<i64>,
    /// Défense.
    #[serde(default)]
    pub def: Option<i64>,
    /// Niveau / rang.
    #[serde(default)]
    pub level: Option<i64>,
    /// Attribut — `null` pour les magies et les pièges.
    #[serde(default)]
    pub attribute: Option<String>,
    /// Type (Dragon, Spellcaster…).
    #[serde(default)]
    pub race: String,
    /// Illustrations — **seule la première est lue**.
    #[serde(default)]
    pub card_images: Vec<ImageApi>,
    /// Tirages, tous sets confondus.
    #[serde(default)]
    pub card_sets: Vec<TirageApi>,
}

/// Une illustration YGOPRODeck.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ImageApi {
    /// URL pleine résolution.
    #[serde(default)]
    pub image_url: String,
    /// URL réduite.
    #[serde(default)]
    pub image_url_small: String,
}

/// Un tirage YGOPRODeck.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TirageApi {
    /// Nom du set — sert à écarter les tirages des autres sets.
    #[serde(default)]
    pub set_name: String,
    /// Code de set complet.
    #[serde(default)]
    pub set_code: String,
    /// Libellé de rareté.
    #[serde(default)]
    pub set_rarity: String,
    /// Code de rareté, **entre parenthèses** dans la réponse.
    #[serde(default)]
    pub set_rarity_code: String,
}

/// Nom complet d'un set, depuis son préfixe.
///
/// Portage de `_find_set_name`. `cardinfo.php` n'accepte pas le préfixe : il
/// faut le nom complet, d'où ce détour par `cardsets.php`.
pub fn nom_du_set<'a>(code_set: &str, sets: &'a [SetYgoprodeck]) -> Option<&'a str> {
    let cherche = code_set.to_uppercase();
    sets.iter()
        .find(|s| s.set_code.to_uppercase() == cherche)
        .map(|s| s.set_name.as_str())
}

/// Construit les lignes d'un classeur depuis une réponse YGOPRODeck.
///
/// Portage de la partie **pure** de `_fetch_rows_from_api` : le réseau est à
/// l'appelant, la transformation est ici.
///
/// # Ce que l'API ne sait pas donner
///
/// `card_uuid` et `card_image_uuid` restent **vides** : ce sont des
/// identifiants YGOJSON, dont YGOPRODeck n'a pas connaissance. Les lignes n'en
/// restent pas moins écrivables — c'est pour ça que le Python les met à vide
/// plutôt que de changer de structure.
///
/// En contrepartie, l'API donne `rarity_code`, que le chemin local laisse vide.
/// Les parenthèses dont elle l'entoure — `(UR)` — sont retirées.
///
/// # Une ligne par carte **et par rareté**
///
/// Une carte qui sort en sept raretés donne sept lignes. Les tirages d'autres
/// sets figurant dans la même réponse sont écartés par comparaison du nom de
/// set — d'où l'importance d'avoir le nom exact.
pub fn lignes_depuis_api(
    nom_set: &str,
    cartes_en: &[CarteApi],
    cartes_fr: &[CarteApi],
) -> Vec<LigneClasseur> {
    // Noms français, indexés par identifiant.
    let noms_fr: HashMap<i64, &str> = cartes_fr
        .iter()
        .filter_map(|c| match (c.id, c.name.as_str()) {
            (Some(id), nom) if id != 0 && !nom.is_empty() => Some((id, nom)),
            _ => None,
        })
        .collect();

    let mut lignes = Vec::new();
    for carte in cartes_en {
        let premiere = carte.card_images.first();
        let image_url = premiere.map(|i| i.image_url.clone()).unwrap_or_default();
        let image_small = premiere
            .map(|i| i.image_url_small.clone())
            .unwrap_or_default();

        for tirage in &carte.card_sets {
            if tirage.set_name != nom_set {
                continue;
            }
            lignes.push(LigneClasseur {
                // Identifiants YGOJSON : l'API ne les connaît pas.
                card_uuid: String::new(),
                card_image_uuid: String::new(),
                card_image_id: carte.id,
                name: carte.name.clone(),
                name_fr: carte
                    .id
                    .and_then(|id| noms_fr.get(&id))
                    .map(|n| (*n).to_owned())
                    .unwrap_or_default(),
                set_code: tirage.set_code.clone(),
                rarity: tirage.set_rarity.clone(),
                rarity_code: tirage.set_rarity_code.trim_matches(['(', ')']).to_owned(),
                set_name: nom_set.to_owned(),
                card_image_url: image_url.clone(),
                card_image_small: image_small.clone(),
                sort_order: ordre_depuis_code(&tirage.set_code),
                card_type: carte.type_carte.clone(),
                atk: carte.atk,
                def_val: carte.def,
                level: carte.level,
                attribute: carte.attribute.clone(),
                race: carte.race.clone(),
                // L'API ignore l'Overframe : c'est la passe dédiée qui le pose.
                extended_art: 0,
            });
        }
    }

    trier_et_numeroter(&mut lignes);
    lignes
}

// ─────────────────────────────────────────────────────────────────────────────
// Écriture du classeur
// ─────────────────────────────────────────────────────────────────────────────

/// Le classeur existe-t-il **et** contient-il au moins une carte ?
///
/// Portage de `classeur_db_est_peuple`. La nuance compte : un dossier
/// **résiduel** — base absente, vide, ou fichiers WAL orphelins d'une
/// suppression partielle sous Windows — ne doit pas empêcher de recréer le
/// classeur. C'est ce qui permet de retélécharger un classeur juste après
/// l'avoir supprimé.
///
/// Toute erreur d'accès vaut `false` : base corrompue ou verrouillée, on
/// considère qu'il n'y a rien à préserver.
pub fn classeur_est_peuple(chemin_db: &Path) -> bool {
    if !chemin_db.is_file() {
        return false;
    }
    let Ok(conn) = Connection::open(chemin_db) else {
        return false;
    };
    let a_la_table = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='cards'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !a_la_table {
        return false;
    }
    conn.query_row("SELECT COUNT(*) FROM cards", [], |l| l.get::<_, i64>(0))
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Crée la base d'un classeur et y insère les lignes.
///
/// Portage de `_save_classeur_from_rows_local`, moins ses trois crochets —
/// correction de rareté, migration défensive, application des anomalies — qui
/// appartiennent à des modules pas encore portés.
///
/// Le schéma vient du DDL **extrait de la base réelle**
/// ([`ygo_db::schema::DDL_CLASSEUR`]) et non d'un `CREATE TABLE` recopié : le
/// Python en porte une quatrième copie ici, ce qui est exactement la
/// duplication que le portage supprime.
///
/// Dix-neuf colonnes sont écrites ; les cinq autres — possession, quantité,
/// qualité, édition, ajout manuel — prennent leurs valeurs par défaut. C'est
/// bien voulu : un classeur neuf ne possède rien.
pub fn ecrire_classeur(
    chemin_db: &Path,
    lignes: &[LigneClasseur],
    ecarts: &crate::classeur::Ecarts,
) -> Result<usize> {
    if let Some(parent) = chemin_db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(chemin_db)?;
    let tx = conn.transaction()?;
    ygo_db::schema::creer_tout(&tx, ygo_db::schema::DDL_CLASSEUR)?;

    let mut ecrites = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO cards
                (card_uuid, card_image_uuid, card_image_id, set_code, rarity,
                 rarity_code, set_name, name, name_fr, card_image_url,
                 card_image_small, sort_order, card_type, atk, def_val, level,
                 attribute, race, extended_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19)",
        )?;
        for l in lignes {
            stmt.execute(rusqlite::params![
                &l.card_uuid,
                &l.card_image_uuid,
                l.card_image_id,
                &l.set_code,
                &l.rarity,
                &l.rarity_code,
                &l.set_name,
                &l.name,
                &l.name_fr,
                &l.card_image_url,
                &l.card_image_small,
                l.sort_order,
                &l.card_type,
                l.atk,
                l.def_val,
                l.level,
                &l.attribute,
                &l.race,
                l.extended_art,
            ])?;
            ecrites += 1;
        }
    }
    // Dans la **même** transaction que les cartes : un classeur et la note
    // qui explique sa forme ne peuvent pas diverger.
    crate::classeur::enregistrer_ecarts_sur(&tx, ecarts)?;
    tx.commit()?;
    Ok(ecrites)
}

// ─────────────────────────────────────────────────────────────────────────────
// Orchestration
// ─────────────────────────────────────────────────────────────────────────────

/// Ce que la création décide de faire, avant d'agir.
///
/// Le Python enchaîne cette décision et son exécution dans une seule fonction
/// où le réseau, le disque et la logique se mêlent. La séparer rend la table de
/// décision lisible — et testable sans toucher ni au réseau ni au disque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Le classeur existe déjà et contient des cartes : on n'y touche pas.
    DejaExistant,
    /// Écrire depuis les lignes locales.
    DepuisLocal,
    /// Passer par YGOPRODeck.
    DepuisApi {
        /// Si l'API échoue, écrire quand même depuis le local.
        ///
        /// Vrai quand les lignes locales existent mais paraissent
        /// incomplètes : un classeur partiel vaut mieux que pas de classeur.
        /// Faux quand le set est absent en local — il n'y a alors rien sur
        /// quoi se rabattre.
        repli_local: bool,
    },
    /// Aucune source praticable.
    Impossible(String),
}

/// Décide comment créer un classeur.
///
/// Portage de la cascade de `_create_classeur_base`.
///
/// # Pourquoi l'OCG n'a pas de repli
///
/// L'API YGOPRODeck indexe les sets par **préfixe nu** (`LOCH`), pas par
/// préfixe et langue (`LOCH-JP`). Un classeur japonais dont le set manque à
/// `cardinfo.db` n'a donc aucune source de secours, et mieux vaut le dire que
/// remplir le classeur de codes anglais.
///
/// # Pourquoi on ne compare pas les deux sources
///
/// La tentation, quand le local paraît incomplet, est de bâtir les deux et de
/// garder la plus fournie. Le Python s'y refuse explicitement, et il a raison :
/// YGOJSON contient des tirages **hallucinés** pour de nombreux sets — il
/// rattache à un set tous les artworks alternatifs connus d'une carte, y
/// compris ceux qui n'y sont pas physiquement. Préférer le local donnerait
/// `LDK2` à 189 cartes au lieu de 132, et trois Obelisk là où il y en a un.
///
/// Le compromis retenu est de perdre quelques cartes — 130 contre 132 sur
/// `LDK2` — plutôt que d'introduire des cartes fantômes dans des dizaines de
/// classeurs. Les artworks manquants se rajoutent à la main.
pub fn decider(
    classeur_peuple: bool,
    code_set: &str,
    lignes_locales: std::result::Result<&[LigneClasseur], String>,
) -> Decision {
    if classeur_peuple {
        return Decision::DejaExistant;
    }
    let ocg = a_suffixe_ocg(code_set);

    // Une liste vide vaut une absence : le Python remet explicitement
    // `rows_local` à `None` dans ce cas, pour que la cascade se déroule
    // pareil qu'avec un set introuvable.
    let lignes = match lignes_locales {
        Ok(lignes) if !lignes.is_empty() => lignes,
        Ok(_) => {
            return manquant(
                ocg,
                code_set,
                &format!("set « {code_set} » présent mais aucune carte exploitable"),
            )
        }
        Err(raison) => return manquant(ocg, code_set, &raison),
    };

    // Le set est là. Reste à savoir si ses données tiennent debout.
    if locales_semblent_incompletes(lignes).suspect && !ocg {
        return Decision::DepuisApi { repli_local: true };
    }
    Decision::DepuisLocal
}

/// Le set n'est pas exploitable en local : API, ou rien du tout.
fn manquant(ocg: bool, code_set: &str, raison: &str) -> Decision {
    if ocg {
        Decision::Impossible(format!(
            "classeur OCG « {code_set} » impossible à créer :\n\
             - cardinfo.db : {raison}\n\
             - l'API YGOPRODeck ne prend pas en charge le format OCG-JP.\n\
             Lancez « MAJ BDD » — Options ▸ Base de référence — pour la mettre à jour."
        ))
    } else {
        Decision::DepuisApi { repli_local: false }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// L'orchestrateur
// ─────────────────────────────────────────────────────────────────────────────

/// D'où viennent les lignes écrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `cardinfo.db` — le chemin normal, instantané et hors ligne.
    Local,
    /// YGOPRODeck — le repli.
    Api,
    /// YGOPRODeck a échoué, on est retombé sur des lignes locales imparfaites.
    ///
    /// Le Python le journalise en avertissement : « refaites MAJ BDD plus tard
    /// quand YGOJSON sera à jour ». Un classeur partiel vaut mieux qu'aucun.
    LocalApresEchecApi,
}

/// Ce qu'est devenue la passe artworks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artworks {
    /// Elle a tourné.
    Faite(crate::artworks::Bilan),
    /// Elle a échoué — sans conséquence sur la création.
    Ignoree(String),
    /// L'appelant ne l'a pas demandée.
    NonDemandee,
}

/// Ce que la création a produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// Le classeur a été créé.
    Cree {
        /// D'où viennent les lignes.
        source: Source,
        /// Combien de lignes ont été écrites.
        lignes: usize,
        /// Ce qu'a donné la passe artworks.
        artworks: Artworks,
        /// Ce qu'a donné la canonisation des libellés de rareté.
        raretes: crate::raretes::BilanCreation,
    },
    /// Le classeur existait déjà et contenait des cartes : rien n'a été touché.
    DejaExistant,
}

/// Les étapes du Python qui ne sont pas encore portées.
///
/// # Pourquoi elles sont ici plutôt que nulle part
///
/// `_save_classeur_from_rows_local` appelle trois greffons après l'écriture,
/// **chacun enveloppé dans un `try/except` qui journalise et continue** :
///
/// | greffon Python | ce qu'il fait | module attendu |
/// |---|---|---|
/// | `corriger_rows` | corrige via Yugipedia une rareté que YGOPRODeck a remplacée par une annotation (`CH02-EN001` : « New artwork » au lieu d'« Ultra Rare ») | `ygo-app::rarity_fix` |
/// | `reparer_classeur` | rectifie set_codes non-EN et raretés numériques — **documenté comme un no-op** sur un classeur fraîchement créé | `ygo-app::maintenance` |
/// | `appliquer_overrides_sur_classeur_neuf` | applique les overrides d'anomalies connus | `ygo-app::anomalies` |
///
/// Ne rien brancher revient **exactement** au chemin d'exception que le Python
/// prévoit déjà — Yugipedia injoignable, module absent. Ce n'est donc pas une
/// divergence de comportement, mais un comportement dégradé que le Python
/// connaît. Le dire ici plutôt que de l'oublier : quand `rarity_fix` et
/// `anomalies` seront portés, ils se branchent là, et ce commentaire disparaît.
#[derive(Default)]
pub struct Greffons<'a> {
    /// Correction des raretés avant écriture.
    pub corriger_raretes: Option<&'a CorrecteurRaretes<'a>>,
    /// Overrides d'anomalies après écriture.
    pub appliquer_overrides: Option<&'a (dyn Fn(&Path) + 'a)>,
}

/// Corrige les raretés d'un lot de lignes avant écriture.
///
/// # La durée de vie n'est pas décorative
///
/// `+ 'a` est indispensable. Écrite en ligne, `&'a dyn Fn(…)` prend `'a` comme
/// borne d'objet ; passée par un **alias de type**, la borne par défaut
/// redevient `'static`, et plus aucune fermeture capturant une variable locale
/// ne peut être branchée — ce qui rendrait ce point d'extension inutilisable
/// depuis un test comme depuis l'application.
pub type CorrecteurRaretes<'a> = dyn Fn(&str, &mut Vec<LigneClasseur>) + 'a;

impl std::fmt::Debug for Greffons<'_> {
    /// Des fonctions ne s'affichent pas : on dit lesquelles sont branchées.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Greffons")
            .field("corriger_raretes", &self.corriger_raretes.is_some())
            .field("appliquer_overrides", &self.appliquer_overrides.is_some())
            .finish()
    }
}

/// Crée le classeur d'un set, artworks compris.
///
/// Portage de `create_classeur` — l'enchaînement de `_create_classeur_base` et
/// de `completer_artworks_variantes`.
///
/// # La séparation qui protège la création
///
/// La passe artworks demande le réseau ; la création, non. Le Python les
/// sépare pour qu'un échec Yugipedia ne puisse **jamais** compromettre le
/// classeur, et absorbe son exception. Le portage garde la séparation et rend
/// l'échec **visible** dans [`Artworks::Ignoree`] au lieu de le journaliser :
/// un classeur créé sans ses variantes n'est pas la même chose qu'un classeur
/// créé avec, et l'appelant a le droit de le savoir.
///
/// # Le dossier résiduel
///
/// Un classeur **peuplé** n'est jamais recréé. Un dossier **résiduel** — base
/// absente, vide, ou fichiers WAL orphelins après une suppression partielle —
/// ne doit en revanche pas bloquer : il est purgé, puis recréé. C'est ce qui
/// permet de retélécharger un classeur juste après l'avoir supprimé.
pub async fn creer(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code_set: &str,
    avec_artworks: bool,
    greffons: &Greffons<'_>,
) -> Result<Issue> {
    let code = code_set.trim().to_uppercase();
    let dossier = paths.dossier_classeur(&code);
    let chemin_db = paths.classeur_db(&code);

    // ── Court-circuit et purge ──────────────────────────────────────────
    if dossier.exists() {
        if classeur_est_peuple(&chemin_db) {
            return Ok(Issue::DejaExistant);
        }
        if let Err(e) = std::fs::remove_dir_all(&dossier) {
            // Le Python journalise et continue : `create_dir_all` plus bas
            // réussira si le dossier est simplement vide.
            tracing::warn!(classeur = %code, erreur = %e, "purge du dossier résiduel impossible");
        }
    }

    // ── Décider ─────────────────────────────────────────────────────────
    let base = paths.cardinfo_db();
    let locales = match ygo_db::connexion::ouvrir_lecture_seule(&base) {
        Ok(conn) => construire_lignes_locales(&conn, &code).map_err(|e| e.to_string()),
        Err(e) => Err(format!("cardinfo.db illisible : {e}")),
    };
    let decision = decider(false, &code, locales.as_deref().map_err(Clone::clone));

    // ── Produire les lignes ─────────────────────────────────────────────
    //
    // L'appel réseau est ici, et l'arbitrage juste après, dans une fonction
    // pure : c'est ce découpage qui rend le repli éprouvable sans réseau.
    let api = match decision {
        Decision::DepuisApi { .. } => {
            Some(depuis_api(client, &code).await.map_err(|e| e.to_string()))
        }
        _ => None,
    };
    let (mut lignes, source) = match arbitrer(&code, decision, locales, api)? {
        Some(couple) => couple,
        None => return Ok(Issue::DejaExistant),
    };

    // ── Écrire ──────────────────────────────────────────────────────────
    if let Some(corriger) = greffons.corriger_raretes {
        corriger(&code, &mut lignes);
    }

    // ── Canonisation des raretés — ÉCART DÉLIBÉRÉ AU PORTAGE ────────────
    //
    // Le Python écrit le libellé tel que la source le lui donne : « UR » de
    // Yugipedia, « PLatinum Secret Rare » d'YGOPRODeck. Ces libellés tombent
    // ensuite sur `PRIORITE_INCONNUE_TRI = 9999` et
    // `PRIORITE_INCONNUE_FILTRE = 0`, c'est-à-dire au mauvais endroit du
    // classeur. Sur demande de l'utilisateur, on les ramène ici à la forme de
    // la liste des Options, **avant** l'écriture, pour que rien d'inexact
    // n'entre en base.
    //
    // Ce qui n'est reconnu par aucune table est laissé tel quel et journalisé
    // — jamais rapproché au plus proche. C'est ce refus qui a valeur de règle :
    // `force-SMW`, sur `RA05-EN136`, n'est pas une rareté.
    let reference = ygo_core::rarity::Priorites::charger(paths.rarity_config());
    let bilan_raretes = crate::raretes::canoniser_lignes(&mut lignes, &reference);
    if bilan_raretes.corrigees > 0 {
        tracing::info!(
            classeur = %code,
            corrigees = bilan_raretes.corrigees,
            "libellés de rareté canonisés"
        );
    }
    for (libelle, n) in &bilan_raretes.inconnues {
        tracing::warn!(
            classeur = %code,
            libelle = %libelle,
            lignes = n,
            "libellé de rareté non reconnu — laissé tel quel"
        );
    }

    // ── Les fausses raretés ─────────────────────────────────────────────
    //
    // Ce qui reste non reconnu **et** partage son illustration avec une
    // rareté véritable n'est pas une rareté : c'est un marqueur d'artwork
    // qu'YGOPRODeck range dans `set_rarity` (« New », « New artwork » sur
    // CH01 et CH02). La règle est dans `raretes::ecarter_fantomes`, avec sa
    // mesure ; elle ne touche pas à `force-SMW`, seul sur son image.
    let fantomes = crate::raretes::ecarter_fantomes(&mut lignes, &reference);
    for fantome in &fantomes {
        tracing::warn!(
            classeur = %code,
            carte = %fantome.set_code,
            libelle = %fantome.rarete,
            jumelles = %fantome.jumelles.join(", "),
            "fausse rareté écartée"
        );
    }

    // Ce qu'on vient de corriger ou de refuser est consigné **dans** le
    // classeur : l'écran en montrera une ligne. Un écart qu'on ne peut pas
    // constater après coup vaut un écart caché.
    let ecarts = crate::classeur::Ecarts {
        quand: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        canonisees: bilan_raretes.corrigees,
        fantomes,
        inconnues: crate::raretes::inconnues(&lignes, &reference),
    };
    let ecrites = ecrire_classeur(&chemin_db, &lignes, &ecarts)?;
    if let Some(overrides) = greffons.appliquer_overrides {
        overrides(&chemin_db);
    }

    // ── L'image Yugipedia de chaque tirage ──────────────────────────────
    //
    // Posée tout de suite, pour que le classeur soit juste même sans passe
    // artworks ; reposée après elle, qui crée des tirages en recopiant l'URL
    // d'une ligne sœur (cf. `completer_artworks`).
    poser_images_tirage(paths, &code, &reference);

    // ── La passe artworks, et le rattrapage qui la suit ─────────────────
    //
    // `avec_artworks` est **faux** quand l'appel vient de l'interface : elle
    // rend la main dès les lignes écrites, puis lance la passe elle-même, par
    // [`completer_artworks`]. Le rattrapage des fausses raretés vit donc là,
    // avec la passe qui le rend nécessaire — et non ici, où il ne verrait
    // jamais les tirages qu'elle ajoute.
    let mut ecrites = ecrites;
    let artworks = if avec_artworks {
        match completer_artworks(paths, client, &code, &reference).await {
            Ok(apres) => {
                ecrites = ecrites.saturating_sub(apres.fantomes_retires);
                Artworks::Faite(apres.bilan)
            }
            Err(e) => Artworks::Ignoree(e.to_string()),
        }
    } else {
        Artworks::NonDemandee
    };

    Ok(Issue::Cree {
        source,
        lignes: ecrites,
        artworks,
        raretes: bilan_raretes,
    })
}

/// Ajoute aux écarts déjà consignés ce que le second passage a trouvé.
///
/// Les libellés inconnus sont **recomptés dans la base**, pas additionnés : le
/// premier passage les a comptés quand les fausses raretés y étaient encore.
/// Les laisser tels quels faisait dire au bandeau « 2 libellés non reconnus »
/// sur un classeur qui n'en contient plus aucun — mesuré sur `CH01` et `CH02`
/// le 2026-09-30. Les lignes gardées parce que possédées, elles, y sont
/// encore : le recompte les trouve sans qu'on les ajoute à la main.
fn consigner_second_passage(
    chemin_db: &Path,
    bilan: &crate::raretes::BilanSecondPassage,
    reference: &ygo_core::rarity::Priorites,
) {
    let mut ecarts = crate::classeur::ecarts_du_classeur(chemin_db).unwrap_or_default();
    if ecarts.quand.is_empty() {
        ecarts.quand = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }
    ecarts.fantomes.extend(bilan.ecartees.iter().cloned());
    // Connexion simple, pour la même raison qu'au-dessus : pas de `-wal`
    // laissé derrière sur un classeur qu'on vient de refermer.
    let consigne = Connection::open(chemin_db)
        .map_err(AppError::from)
        .and_then(|conn| {
            ecarts.inconnues = crate::raretes::analyser(&conn, reference)?.inconnues;
            crate::classeur::enregistrer_ecarts_sur(&conn, &ecarts)
        });
    if let Err(e) = consigne {
        tracing::warn!(erreur = %e, "écarts du second passage non consignés");
    }
}

/// Choisit quelles lignes écrire, une fois les sources interrogées.
///
/// Extraction de la cascade de `_create_classeur_base`, **sans le réseau** :
/// l'appelant a déjà interrogé YGOPRODeck si la décision le demandait, et
/// passe son résultat tel quel. C'est ce qui rend la branche de repli
/// éprouvable — elle est autrement inatteignable sans couper Internet.
///
/// Rend `None` quand il n'y a rien à écrire (classeur déjà existant).
///
/// # Le repli, et pourquoi il n'est pas symétrique
///
/// Quand l'API échoue :
///
/// - `repli_local` **vrai** — les lignes locales existent mais paraissent
///   incomplètes : on les écrit quand même. Un classeur partiel vaut mieux
///   qu'aucun classeur, et l'utilisateur pourra refaire « MAJ BDD » plus tard ;
/// - `repli_local` **faux** — le set est absent en local : il n'y a rien sur
///   quoi se rabattre, et l'erreur énumère ce que chaque source a répondu.
fn arbitrer(
    code_set: &str,
    decision: Decision,
    locales: std::result::Result<Vec<LigneClasseur>, String>,
    api: Option<std::result::Result<Vec<LigneClasseur>, String>>,
) -> Result<Option<(Vec<LigneClasseur>, Source)>> {
    match decision {
        Decision::DejaExistant => Ok(None),
        Decision::Impossible(raison) => Err(AppError::Creation(raison)),
        Decision::DepuisLocal => Ok(Some((locales.map_err(AppError::Creation)?, Source::Local))),
        Decision::DepuisApi { repli_local } => match api {
            Some(Ok(lignes)) => Ok(Some((lignes, Source::Api))),
            Some(Err(e)) if repli_local => {
                tracing::warn!(
                    classeur = %code_set, erreur = %e,
                    "API injoignable — création depuis le local malgré des données partielles"
                );
                Ok(Some((
                    locales.map_err(AppError::Creation)?,
                    Source::LocalApresEchecApi,
                )))
            }
            Some(Err(e)) => Err(AppError::Creation(format!(
                "classeur « {code_set} » impossible à créer :\n\
                 - cardinfo.db : {}\n\
                 - API YGOPRODeck : {e}",
                locales.err().unwrap_or_else(|| "set absent".to_owned())
            ))),
            // La décision demandait l'API et l'appelant ne l'a pas interrogée.
            None => Err(AppError::Creation(format!(
                "classeur « {code_set} » : YGOPRODeck n'a pas été interrogé"
            ))),
        },
    }
}

/// Les lignes d'un set, depuis YGOPRODeck.
///
/// Portage de `_fetch_rows_from_api`. Deux requêtes après la liste des sets :
/// l'anglaise, dont l'échec est fatal, et la française, dont l'échec ne l'est
/// pas — un nom français manquant n'empêche pas de créer un classeur.
async fn depuis_api(
    client: &ygo_sources::ClientHttp,
    code_set: &str,
) -> Result<Vec<LigneClasseur>> {
    let sets: Vec<SetYgoprodeck> = ygo_sources::ygoprodeck::lister_sets(client)
        .await
        .map_err(|e| AppError::Creation(format!("liste des sets YGOPRODeck : {e}")))?;

    let Some(nom) = nom_du_set(code_set, &sets) else {
        return Err(AppError::Creation(format!(
            "code de set « {code_set} » introuvable chez YGOPRODeck.\n\
             Vérifiez qu'il est correct (LOB, RA05, SS01…)."
        )));
    };
    let nom = nom.to_owned();

    let cartes_en: Vec<CarteApi> = ygo_sources::ygoprodeck::cartes_du_set(client, &nom, "")
        .await
        .map_err(|e| AppError::Creation(format!("cartes de « {nom} » : {e}")))?;
    if cartes_en.is_empty() {
        return Err(AppError::Creation(format!(
            "aucune carte trouvée pour le set « {code_set} » ({nom})."
        )));
    }

    // Le français est un bonus : son échec ne doit pas coûter le classeur.
    let cartes_fr: Vec<CarteApi> = ygo_sources::ygoprodeck::cartes_du_set(client, &nom, "fr")
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(set = %nom, erreur = %e, "noms français indisponibles");
            Vec::new()
        });

    Ok(lignes_depuis_api(&nom, &cartes_en, &cartes_fr))
}

/// Ce que la passe artworks laisse derrière elle, rattrapage compris.
#[derive(Debug, Clone)]
pub struct ApresArtworks {
    /// Le bilan de la passe elle-même.
    pub bilan: crate::artworks::Bilan,
    /// Lignes retirées par le rattrapage des fausses raretés.
    pub fantomes_retires: usize,
}

/// La passe artworks d'un classeur, **suivie** du rattrapage des fantômes.
///
/// # Pourquoi les deux ensemble — 2026-09-30
///
/// La passe artworks crée les tirages manquants d'un numéro en **héritant**
/// des colonnes d'une ligne existante, `card_image_id` compris. Un fantôme
/// resté seul sur son illustration à l'écriture — que
/// [`crate::raretes::ecarter_fantomes`] laisse passer, faute de jumelle
/// reconnue — devient donc, à la fin de cette passe, une ligne entourée de
/// vraies raretés sur la même image. C'est là, et seulement là, que la règle
/// peut trancher.
///
/// Le premier correctif plaçait ce rattrapage dans [`creer`], après son propre
/// appel à la passe. L'interface ne passe jamais par ce chemin : elle appelle
/// `creer` avec `avec_artworks = false` pour rendre la main tôt, puis lance la
/// passe de son côté. Le rattrapage ne voyait donc jamais un seul tirage
/// ajouté — mesuré sur `CH01` et `CH02` recréés le 30 septembre : 64 lignes au
/// lieu des 62 de Yugipedia, exactement comme avant le correctif. Les deux
/// gestes sont ici réunis pour que le chemin ne puisse plus se scinder.
///
/// # Errors
///
/// Rend une erreur si la passe artworks échoue. Le rattrapage, lui, ne fait
/// jamais échouer l'appel : un classeur écrit vaut mieux qu'un classeur perdu.
pub async fn completer_artworks(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
    reference: &ygo_core::rarity::Priorites,
) -> Result<ApresArtworks> {
    let chemin_db = paths.classeur_db(code);
    let chemin_db = chemin_db.as_path();
    let bilan = {
        let conn = ygo_db::connexion::ouvrir(chemin_db)?;
        match crate::artworks::passe(&conn, client, "", reference).await? {
            crate::artworks::Issue::Fait { bilan, .. } => bilan,
            crate::artworks::Issue::ClasseurVide => {
                return Err(AppError::Creation(
                    "classeur vide au moment de la passe artworks".to_owned(),
                ))
            }
            crate::artworks::Issue::PageIntrouvable => {
                return Err(AppError::Creation(
                    "aucune page « Set Card Lists » pour ce set".to_owned(),
                ))
            }
            crate::artworks::Issue::StructureSterile { titre } => {
                return Err(AppError::Creation(format!(
                    "{titre} — aucun tirage exploitable"
                )))
            }
        }
    };
    let fantomes_retires = rattraper_fantomes(chemin_db, code, reference);
    // La passe a créé des tirages en recopiant une ligne sœur, URL comprise :
    // chacun doit recevoir l'image de **son** tirage.
    poser_images_tirage(paths, code, reference);
    Ok(ApresArtworks {
        bilan,
        fantomes_retires,
    })
}

/// Pose l'image Yugipedia de chaque tirage, et le journalise.
///
/// Ne rend jamais d'erreur : un classeur dont les images restent celles
/// d'YGOPRODeck est un classeur utilisable — c'était le cas de tous jusqu'ici.
/// Cf. [`crate::images_tirage`].
fn poser_images_tirage(paths: &Paths, code: &str, reference: &ygo_core::rarity::Priorites) {
    match crate::images_tirage::poser(&paths.classeur_db(code), &paths.cardinfo_db(), reference) {
        Ok(r) => tracing::info!(
            classeur = %code,
            lignes = r.lues,
            posees = r.a_poser.len() - r.rendues,
            deja = r.deja,
            rendues = r.rendues,
            sans_tirage = r.sans_tirage,
            sans_image = r.sans_image,
            ambigues = r.ambigues,
            "images de tirage Yugipedia"
        ),
        Err(e) => tracing::warn!(classeur = %code, erreur = %e, "images de tirage non posées"),
    }
}

/// Rejoue la règle des fausses raretés sur le classeur écrit, et la consigne.
///
/// Rend le nombre de lignes retirées. Ne rend jamais d'erreur : un second
/// passage empêché laisse le classeur tel quel, ce qui reste utilisable.
fn rattraper_fantomes(
    chemin_db: &Path,
    code: &str,
    reference: &ygo_core::rarity::Priorites,
) -> usize {
    match crate::raretes::ecarter_fantomes_en_base(chemin_db, reference) {
        Ok(bilan) if bilan.vide() => 0,
        Ok(bilan) => {
            for fantome in bilan.ecartees.iter().chain(&bilan.gardees) {
                tracing::warn!(
                    classeur = %code,
                    carte = %fantome.set_code,
                    libelle = %fantome.rarete,
                    jumelles = %fantome.jumelles.join(", "),
                    "fausse rareté vue au second passage"
                );
            }
            let retirees = bilan.ecartees.len();
            consigner_second_passage(chemin_db, &bilan, reference);
            retirees
        }
        Err(e) => {
            tracing::warn!(classeur = %code, erreur = %e, "second passage impossible");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Une `cardinfo.db` réduite, bâtie sur le **vrai DDL** — pas sur une
    /// transcription. Les requêtes portées s'exécutent donc contre le schéma
    /// de production, avec ses types et ses contraintes.
    fn base() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Comme en production : les données amont portent de vraies violations
        // d'intégrité que la V1.0.4 insère sans broncher.
        let _ = conn.pragma_update(None, "foreign_keys", "OFF");
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CARDINFO).unwrap();
        conn
    }

    struct Constructeur<'a> {
        conn: &'a Connection,
        locale: i64,
    }

    impl<'a> Constructeur<'a> {
        fn set(conn: &'a Connection, uuid: &str, nom_en: &str) -> Self {
            conn.execute(
                "INSERT INTO sets (uuid, name_en) VALUES (?1, ?2)",
                (uuid, nom_en),
            )
            .unwrap();
            Self { conn, locale: 0 }
        }

        /// Ajoute une déclinaison de langue et la rend courante.
        fn locale(mut self, set_uuid: &str, langue: &str, prefixe: &str) -> Self {
            self.conn
                .execute(
                    "INSERT INTO set_locales (set_uuid, language, prefix) VALUES (?1, ?2, ?3)",
                    (set_uuid, langue, prefixe),
                )
                .unwrap();
            self.locale = self.conn.last_insert_rowid();
            self
        }

        /// Ajoute une carte, son texte anglais et français, et son illustration.
        fn carte(self, uuid: &str, nom_en: &str, nom_fr: &str) -> Self {
            self.conn
                .execute(
                    "INSERT INTO cards (uuid, card_type, subcategory, name_fr_confirmed)
                     VALUES (?1, 'monster', '', 0)",
                    [uuid],
                )
                .unwrap();
            for (langue, nom) in [("en", nom_en), ("fr", nom_fr)] {
                if !nom.is_empty() {
                    self.conn
                        .execute(
                            "INSERT INTO card_texts (card_uuid, language, name, effect)
                             VALUES (?1, ?2, ?3, '')",
                            (uuid, langue, nom),
                        )
                        .unwrap();
                }
            }
            self
        }

        fn image(self, uuid: &str, carte: &str, id: i64) -> Self {
            self.conn
                .execute(
                    "INSERT INTO card_images (uuid, card_uuid, ygoprodeck_image_id, art_url, card_url)
                     VALUES (?1, ?2, ?3, 'art', 'carte')",
                    (uuid, carte, id),
                )
                .unwrap();
            self
        }

        /// Ajoute un tirage dans la locale courante.
        fn tirage(self, carte: &str, image: &str, code: &str, rarete: &str, ext: i64) -> Self {
            self.conn
                .execute(
                    "INSERT INTO set_prints
                        (set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                         rarity, edition, qty, extended_art)
                     VALUES ((SELECT set_uuid FROM set_locales WHERE id = ?1),
                             ?1, ?2, ?3, ?4, ?5, 'unlimited', 1, ?6)",
                    (self.locale, carte, image, code, rarete, ext),
                )
                .unwrap();
            self
        }
    }

    /// Set français complet : deux cartes, deux raretés chacune.
    fn set_bilingue(conn: &Connection, n_fr: usize, n_en: usize) {
        let mut b = Constructeur::set(conn, "S1", "Rarity Collection 5")
            .locale("S1", "en", "RA05-EN")
            .carte("C1", "Dark Magician", "Magicien Sombre")
            .carte("C2", "Blue-Eyes", "Dragon Blanc")
            .image("I1", "C1", 100)
            .image("I2", "C2", 200);
        for i in 0..n_en {
            let rarete = if i == 0 { "Common" } else { "Ultra Rare" };
            b = b.tirage("C1", "I1", "RA05-EN001", rarete, 0).tirage(
                "C2",
                "I2",
                "RA05-EN002",
                rarete,
                0,
            );
        }
        let mut b = b.locale("S1", "fr", "RA05-FR");
        for i in 0..n_fr {
            let rarete = if i == 0 { "Commune" } else { "Ultra Rare" };
            b = b.tirage("C1", "I1", "RA05-FR001", rarete, 0).tirage(
                "C2",
                "I2",
                "RA05-FR002",
                rarete,
                0,
            );
        }
    }

    #[test]
    fn une_locale_francaise_aussi_complete_l_emporte() {
        let conn = base();
        set_bilingue(&conn, 2, 2);
        let choix = choisir_locale(&conn, "RA05", "S1").unwrap();
        assert_eq!(choix.mode, ModeEnumeration::Francaise);
        assert!(
            choix.locale_anglaise.is_some(),
            "il faut la jointure anglaise"
        );
    }

    #[test]
    fn une_locale_francaise_partielle_cede_a_l_anglaise() {
        // Le bug RA05 d'avril 2026 : une rareté française sur deux anglaises.
        // Énumérer en français perdrait la moitié du classeur.
        let conn = base();
        set_bilingue(&conn, 1, 2);
        let choix = choisir_locale(&conn, "RA05", "S1").unwrap();
        assert_eq!(choix.mode, ModeEnumeration::Anglaise);
    }

    #[test]
    fn sans_francaise_du_tout_on_enumere_en_anglais() {
        let conn = base();
        set_bilingue(&conn, 0, 2);
        assert_eq!(
            choisir_locale(&conn, "RA05", "S1").unwrap().mode,
            ModeEnumeration::Anglaise
        );
    }

    #[test]
    fn un_classeur_ocg_force_sa_locale_sans_repli() {
        let conn = base();
        Constructeur::set(&conn, "S2", "Limit Over Collection")
            .locale("S2", "en", "LOCR-EN")
            .carte("C9", "Blue-Eyes", "")
            .image("I9", "C9", 900)
            .tirage("C9", "I9", "LOCR-EN001", "Ultra Rare", 0)
            .locale("S2", "jp", "LOCR-JP")
            .tirage("C9", "I9", "LOCR-JP001", "Ultra Rare", 0);

        let choix = choisir_locale(&conn, "LOCR-JP", "S2").unwrap();
        assert_eq!(choix.mode, ModeEnumeration::Japonaise);
        assert_eq!(
            choix.locale_anglaise, None,
            "aucune jointure anglaise : les codes doivent rester japonais"
        );
    }

    #[test]
    fn un_set_ocg_sans_locale_japonaise_est_refuse() {
        let conn = base();
        Constructeur::set(&conn, "S3", "Set").locale("S3", "en", "ABCD-EN");
        let e = choisir_locale(&conn, "ABCD-JP", "S3").unwrap_err();
        assert!(matches!(e, AppError::SetAbsentDeLaBase(_)));
    }

    #[test]
    fn un_set_sans_aucune_locale_pourvue_est_refuse() {
        let conn = base();
        Constructeur::set(&conn, "S4", "Set")
            .locale("S4", "en", "VIDE-EN")
            .locale("S4", "fr", "VIDE-FR");
        let e = choisir_locale(&conn, "VIDE", "S4").unwrap_err();
        assert!(matches!(e, AppError::SetAbsentDeLaBase(_)));
    }

    #[test]
    fn un_set_absent_de_la_base_est_refuse() {
        let conn = base();
        let e = construire_lignes_locales(&conn, "INCONNU").unwrap_err();
        assert!(matches!(e, AppError::SetAbsentDeLaBase(_)));
    }

    #[test]
    fn l_enumeration_francaise_rend_les_codes_anglais() {
        let conn = base();
        set_bilingue(&conn, 2, 2);
        let lignes = construire_lignes_locales(&conn, "RA05").unwrap();
        assert_eq!(lignes.len(), 4);
        assert!(
            lignes.iter().all(|l| l.set_code.starts_with("RA05-EN")),
            "les codes doivent être anglais, obtenu {:?}",
            lignes.iter().map(|l| &l.set_code).collect::<Vec<_>>()
        );
        // Les raretés, elles, restent celles de la locale énumérée.
        assert!(lignes.iter().any(|l| l.rarity == "Commune"));
    }

    #[test]
    fn la_jointure_anglaise_ne_contraint_pas_la_rarete() {
        // Le bug SDWD de mai 2026 : « Common » en français, « Short Print » en
        // anglais. Avec une jointure contrainte sur la rareté, le code anglais
        // était perdu et le classeur gardait « SDWD-FR013 ».
        let conn = base();
        Constructeur::set(&conn, "S5", "Structure Deck")
            .locale("S5", "en", "SDWD-EN")
            .carte("C5", "Card", "Carte")
            .image("I5", "C5", 500)
            .tirage("C5", "I5", "SDWD-EN013", "Short Print", 0)
            .locale("S5", "fr", "SDWD-FR")
            .tirage("C5", "I5", "SDWD-FR013", "Common", 0);

        let lignes = construire_lignes_locales(&conn, "SDWD").unwrap();
        assert_eq!(lignes.len(), 1);
        assert_eq!(
            lignes[0].set_code, "SDWD-EN013",
            "le code anglais doit être retrouvé malgré la rareté différente"
        );
    }

    #[test]
    fn les_metadonnees_traversent_les_jointures() {
        let conn = base();
        set_bilingue(&conn, 2, 2);
        let lignes = construire_lignes_locales(&conn, "RA05").unwrap();
        let Some(l) = lignes.iter().find(|l| l.set_code == "RA05-EN001") else {
            unreachable!("RA05-EN001 doit figurer parmi les lignes construites")
        };
        assert_eq!(l.name, "Dark Magician");
        assert_eq!(l.name_fr, "Magicien Sombre");
        assert_eq!(l.set_name, "Rarity Collection 5");
        assert_eq!(l.card_image_id, Some(100));
        assert_eq!(l.card_image_url, "carte");
        assert_eq!(l.card_image_small, "art");
        assert_eq!(l.card_uuid, "C1");
        assert_eq!(l.card_image_uuid, "I1");
        assert_eq!(l.card_type, "monster");
        assert_eq!(l.rarity_code, "", "colonne propre à YGOPRODeck");
    }

    #[test]
    fn le_rang_suit_le_tri_et_non_l_ordre_sql() {
        let conn = base();
        Constructeur::set(&conn, "S6", "Multi-deck")
            .locale("S6", "en", "L26D-EN")
            .carte("CX", "X", "")
            .carte("CM", "M", "")
            .image("IX", "CX", 1)
            .image("IM", "CM", 2)
            // Insérés dans le désordre : X avant M.
            .tirage("CX", "IX", "L26D-ENX01", "Common", 0)
            .tirage("CM", "IM", "L26D-ENM01", "Common", 0);

        let lignes = construire_lignes_locales(&conn, "L26D").unwrap();
        assert_eq!(lignes[0].set_code, "L26D-ENM01", "M avant X");
        assert_eq!(lignes[1].set_code, "L26D-ENX01");
        assert_eq!((lignes[0].sort_order, lignes[1].sort_order), (0, 1));
    }

    #[test]
    fn l_overframe_est_range_apres_le_cadre_normal() {
        let conn = base();
        Constructeur::set(&conn, "S7", "Limit Over")
            .locale("S7", "jp", "LOCR-JP")
            .carte("C7", "Blue-Eyes", "")
            .image("I7", "C7", 700)
            // L'Overframe est inséré en premier, et en rareté plus « petite ».
            .tirage("C7", "I7", "LOCR-JP001", "Aaa", 1)
            .tirage("C7", "I7", "LOCR-JP001", "Zzz", 0);

        let lignes = construire_lignes_locales(&conn, "LOCR-JP").unwrap();
        assert_eq!(lignes[0].extended_art, 0, "le cadre normal d'abord");
        assert_eq!(lignes[1].extended_art, 1);
    }

    #[test]
    fn le_tri_est_stable_sur_les_ex_aequo() {
        // Deux illustrations du même numéro, même rareté, même cadre : leur
        // ordre d'arrivée doit être conservé.
        let mut lignes: Vec<LigneClasseur> = ["A", "B", "C"]
            .iter()
            .map(|u| LigneClasseur {
                set_code: "RA05-EN001".to_owned(),
                rarity: "Common".to_owned(),
                card_uuid: (*u).to_owned(),
                ..LigneClasseur::default()
            })
            .collect();
        trier_et_numeroter(&mut lignes);
        assert_eq!(
            lignes
                .iter()
                .map(|l| l.card_uuid.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    }

    // ── Écriture ────────────────────────────────────────────────────────────

    fn ligne(code: &str, rarete: &str) -> LigneClasseur {
        LigneClasseur {
            card_uuid: "CU".to_owned(),
            card_image_uuid: "CIU".to_owned(),
            card_image_id: Some(42),
            name: "Dark Magician".to_owned(),
            name_fr: "Magicien Sombre".to_owned(),
            set_code: code.to_owned(),
            rarity: rarete.to_owned(),
            rarity_code: String::new(),
            set_name: "Set".to_owned(),
            card_image_url: "https://exemple/carte.jpg".to_owned(),
            card_image_small: "https://exemple/art.jpg".to_owned(),
            sort_order: 0,
            card_type: "monster".to_owned(),
            atk: Some(2500),
            def_val: Some(2100),
            level: Some(7),
            attribute: Some("DARK".to_owned()),
            race: "Spellcaster".to_owned(),
            extended_art: 0,
        }
    }

    #[test]
    fn le_classeur_ecrit_est_conforme_au_ddl_reel() {
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("RA05").join("RA05.db");
        let n = ecrire_classeur(
            &chemin,
            &[ligne("RA05-EN001", "Common")],
            &crate::classeur::Ecarts::default(),
        )
        .unwrap();
        assert_eq!(n, 1);

        // Le schéma produit doit être celui de la base réelle, index compris.
        let divergences =
            ygo_db::schema::verifier_fichier(&chemin, ygo_db::schema::DDL_CLASSEUR).unwrap();
        assert!(divergences.is_empty(), "{divergences:?}");
    }

    #[test]
    fn les_colonnes_non_ecrites_prennent_leurs_defauts() {
        // Un classeur neuf ne possède rien : c'est le rôle des défauts du DDL.
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("RA05.db");
        ecrire_classeur(
            &chemin,
            &[ligne("RA05-EN001", "Common")],
            &crate::classeur::Ecarts::default(),
        )
        .unwrap();

        let conn = Connection::open(&chemin).unwrap();
        let (possede, quantite, qualite, edition, custom): (
            i64,
            i64,
            Option<String>,
            Option<String>,
            i64,
        ) = conn
            .query_row(
                "SELECT possessed, quantite, qualite, edition, is_custom FROM cards",
                [],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?, l.get(4)?)),
            )
            .unwrap();
        assert_eq!((possede, quantite, custom), (0, 0, 0));
        assert_eq!((qualite, edition), (None, None));
    }

    #[test]
    fn les_dix_neuf_colonnes_ecrites_arrivent_intactes() {
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("RA05.db");
        let attendue = ligne("RA05-EN001", "Ultra Rare");
        ecrire_classeur(
            &chemin,
            std::slice::from_ref(&attendue),
            &crate::classeur::Ecarts::default(),
        )
        .unwrap();

        let conn = Connection::open(&chemin).unwrap();
        let obtenue = conn
            .query_row(
                "SELECT card_uuid, card_image_uuid, card_image_id, set_code, rarity,
                        rarity_code, set_name, name, name_fr, card_image_url,
                        card_image_small, sort_order, card_type, atk, def_val, level,
                        attribute, race, extended_art
                 FROM cards",
                [],
                |l| {
                    Ok(LigneClasseur {
                        card_uuid: l.get(0)?,
                        card_image_uuid: l.get(1)?,
                        card_image_id: l.get(2)?,
                        set_code: l.get(3)?,
                        rarity: l.get(4)?,
                        rarity_code: l.get(5)?,
                        set_name: l.get(6)?,
                        name: l.get(7)?,
                        name_fr: l.get(8)?,
                        card_image_url: l.get(9)?,
                        card_image_small: l.get(10)?,
                        sort_order: l.get(11)?,
                        card_type: l.get(12)?,
                        atk: l.get(13)?,
                        def_val: l.get(14)?,
                        level: l.get(15)?,
                        attribute: l.get(16)?,
                        race: l.get(17)?,
                        extended_art: l.get(18)?,
                    })
                },
            )
            .unwrap();
        assert_eq!(obtenue, attendue);
    }

    #[test]
    fn un_classeur_vide_n_est_pas_peuple() {
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("RA05.db");

        // Base absente.
        assert!(!classeur_est_peuple(&chemin));
        // Base présente mais sans carte : c'est un résidu, pas un classeur.
        ecrire_classeur(&chemin, &[], &crate::classeur::Ecarts::default()).unwrap();
        assert!(!classeur_est_peuple(&chemin));
        // Avec une carte, cette fois.
        let chemin2 = dossier.path().join("RA02.db");
        ecrire_classeur(
            &chemin2,
            &[ligne("RA02-EN001", "Common")],
            &crate::classeur::Ecarts::default(),
        )
        .unwrap();
        assert!(classeur_est_peuple(&chemin2));
    }

    #[test]
    fn un_fichier_qui_n_est_pas_une_base_n_est_pas_peuple() {
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("faux.db");
        std::fs::write(&chemin, b"ceci n'est pas une base SQLite").unwrap();
        assert!(!classeur_est_peuple(&chemin));
    }

    // ── Repli YGOPRODeck ────────────────────────────────────────────────────

    fn carte_api(id: i64, nom: &str, images: &[&str], tirages: &[(&str, &str, &str)]) -> CarteApi {
        CarteApi {
            id: Some(id),
            name: nom.to_owned(),
            type_carte: "Effect Monster".to_owned(),
            atk: Some(1000),
            def: Some(500),
            level: Some(4),
            attribute: Some("DARK".to_owned()),
            race: "Dragon".to_owned(),
            card_images: images
                .iter()
                .map(|u| ImageApi {
                    image_url: format!("{u}.jpg"),
                    image_url_small: format!("{u}_petit.jpg"),
                })
                .collect(),
            card_sets: tirages
                .iter()
                .map(|(set, code, rarete)| TirageApi {
                    set_name: (*set).to_owned(),
                    set_code: (*code).to_owned(),
                    set_rarity: (*rarete).to_owned(),
                    set_rarity_code: "(UR)".to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn seule_la_premiere_illustration_est_retenue() {
        // L'oracle ne peut pas éprouver ce point : la capture réduite ne garde
        // qu'une illustration par carte, si bien que « la première » et « la
        // dernière » y désignent la même. D'où ce test à la main — écrit après
        // qu'une mutation « prendre la dernière » eut survécu à l'oracle.
        let carte = carte_api(
            1,
            "Carte",
            &["premiere", "seconde", "troisieme"],
            &[("Set", "SET-EN001", "Ultra Rare")],
        );
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &[]);
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].card_image_url, "premiere.jpg");
        assert_eq!(lignes[0].card_image_small, "premiere_petit.jpg");
    }

    #[test]
    fn une_carte_sans_illustration_ne_bloque_rien() {
        let carte = carte_api(1, "Carte", &[], &[("Set", "SET-EN001", "Common")]);
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &[]);
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].card_image_url, "");
        assert_eq!(lignes[0].card_image_small, "");
    }

    #[test]
    fn une_ligne_par_rarete_du_set_vise() {
        let carte = carte_api(
            1,
            "Carte",
            &["img"],
            &[
                ("Set", "SET-EN001", "Common"),
                ("Set", "SET-EN001", "Ultra Rare"),
                ("Autre Set", "AUT-EN001", "Secret Rare"),
            ],
        );
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &[]);
        assert_eq!(lignes.len(), 2, "le tirage de l'autre set est écarté");
        assert_eq!(lignes[0].rarity, "Common");
        assert_eq!(lignes[1].rarity, "Ultra Rare");
        assert_eq!(lignes[0].rarity_code, "UR", "les parenthèses tombent");
    }

    /// Le `set_rarity` numéroté d'YGOPRODeck n'atteint jamais la base.
    ///
    /// # Le défaut réel que ce test ferme
    ///
    /// Sur `SDWD`, l'API rend `set_rarity = "3"` pour *Sage with Eyes of Blue*
    /// et `"2"` pour les cinq autres cartes du deck en plusieurs exemplaires,
    /// avec un `set_rarity_code` vide. Le nombre est le `qty` : `cardinfo.db`
    /// donne, pour les mêmes tirages, `rarity = Common` et `qty = 3` / `2`.
    ///
    /// Le chiffre entrait tel quel en base. Il coûtait trois fois : la ligne
    /// se rangeait en fin de classeur (9999 au tri), l'import CSV ne la
    /// retrouvait jamais — le CSV dit `Common`, la base disait `3` —, et la
    /// passe artworks, ne trouvant aucune ligne `Common` pour ce numéro, en
    /// insérait une seconde. L'utilisateur voyait la carte en double, dont un
    /// exemplaire ne se remplissait jamais.
    ///
    /// Ce test suit le **vrai ordre** de `creer` : les lignes de l'API, puis
    /// la canonisation, puis seulement l'écriture.
    #[test]
    fn un_qty_tombe_dans_la_rarete_ne_va_pas_en_base() {
        let carte = carte_api(
            8_240_199,
            "Sage with Eyes of Blue",
            &["img"],
            &[("Structure Deck: Blue-Eyes White Destiny", "SDWD-EN013", "3")],
        );
        let mut lignes = lignes_depuis_api(
            "Structure Deck: Blue-Eyes White Destiny",
            std::slice::from_ref(&carte),
            &[],
        );
        assert_eq!(lignes[0].rarity, "3", "l'API rend bien le chiffre");

        // La canonisation est le dernier filtre avant l'écriture.
        let reference = ygo_core::rarity::Priorites::charger(std::path::Path::new("néant"));
        let bilan = crate::raretes::canoniser_lignes(&mut lignes, &reference);
        assert_eq!(lignes[0].rarity, "Common");
        assert_eq!(bilan.corrigees, 1);
        assert!(
            bilan.inconnues.is_empty(),
            "et ce n'est pas signalé comme un libellé illisible : {:?}",
            bilan.inconnues
        );
    }

    #[test]
    fn le_nom_francais_se_raccroche_par_identifiant() {
        let carte = carte_api(
            42,
            "Dark Magician",
            &["img"],
            &[("Set", "SET-EN001", "Common")],
        );
        let fr = vec![
            CarteApi {
                id: Some(99),
                name: "Autre Carte".to_owned(),
                ..CarteApi::default()
            },
            CarteApi {
                id: Some(42),
                name: "Magicien Sombre".to_owned(),
                ..CarteApi::default()
            },
        ];
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &fr);
        assert_eq!(lignes[0].name_fr, "Magicien Sombre");

        // Sans correspondance française, le champ reste vide — jamais l'anglais.
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &[]);
        assert_eq!(lignes[0].name_fr, "");
    }

    #[test]
    fn les_identifiants_ygojson_ne_sont_pas_inventes() {
        let carte = carte_api(1, "Carte", &["img"], &[("Set", "SET-EN001", "Common")]);
        let lignes = lignes_depuis_api("Set", std::slice::from_ref(&carte), &[]);
        assert_eq!(lignes[0].card_uuid, "");
        assert_eq!(lignes[0].card_image_uuid, "");
        assert_eq!(lignes[0].extended_art, 0, "l'API ignore l'Overframe");
    }

    #[test]
    fn le_rang_est_renumerote_apres_le_tri() {
        // `sort_order` reçoit d'abord le numéro lu dans le code, puis le rang.
        let cartes = vec![
            carte_api(1, "B", &["i"], &[("Set", "SET-EN050", "Common")]),
            carte_api(2, "A", &["i"], &[("Set", "SET-EN002", "Common")]),
        ];
        let lignes = lignes_depuis_api("Set", &cartes, &[]);
        assert_eq!(lignes[0].set_code, "SET-EN002");
        assert_eq!(
            (lignes[0].sort_order, lignes[1].sort_order),
            (0, 1),
            "le rang remplace le numéro initial"
        );
    }

    // ── Décision ────────────────────────────────────────────────────────────

    /// Des lignes complètes : deux raretés par carte, au-dessus du seuil.
    fn lignes_completes() -> Vec<LigneClasseur> {
        (0..4)
            .map(|i| LigneClasseur {
                card_uuid: format!("C{}", i / 2),
                set_code: format!("RA05-EN00{}", i / 2),
                rarity: if i % 2 == 0 { "Common" } else { "Ultra Rare" }.to_owned(),
                ..LigneClasseur::default()
            })
            .collect()
    }

    /// Une rareté par carte : sous le seuil, donc suspect.
    fn lignes_suspectes() -> Vec<LigneClasseur> {
        (0..4)
            .map(|i| LigneClasseur {
                card_uuid: format!("C{i}"),
                set_code: format!("RA05-EN00{i}"),
                rarity: "Common".to_owned(),
                ..LigneClasseur::default()
            })
            .collect()
    }

    #[test]
    fn un_classeur_deja_peuple_n_est_jamais_recree() {
        assert_eq!(
            decider(true, "RA05", Ok(&lignes_completes())),
            Decision::DejaExistant
        );
        // Y compris quand le local a échoué : rien ne justifie d'y toucher.
        assert_eq!(
            decider(true, "RA05", Err("absent".to_owned())),
            Decision::DejaExistant
        );
    }

    #[test]
    fn des_donnees_locales_completes_suffisent() {
        assert_eq!(
            decider(false, "RA05", Ok(&lignes_completes())),
            Decision::DepuisLocal
        );
    }

    #[test]
    fn des_donnees_suspectes_font_tenter_l_api_avec_repli() {
        // Le cas RA05 : on tente l'API, mais si elle tombe on écrit quand même
        // le local — un classeur partiel vaut mieux que pas de classeur.
        assert_eq!(
            decider(false, "RA05", Ok(&lignes_suspectes())),
            Decision::DepuisApi { repli_local: true }
        );
    }

    #[test]
    fn un_set_absent_en_local_passe_a_l_api_sans_repli() {
        assert_eq!(
            decider(false, "RA05", Err("absent de cardinfo.db".to_owned())),
            Decision::DepuisApi { repli_local: false }
        );
        // Une liste vide vaut une absence.
        assert_eq!(
            decider(false, "RA05", Ok(&[])),
            Decision::DepuisApi { repli_local: false }
        );
    }

    #[test]
    fn un_classeur_ocg_suspect_reste_sur_le_local() {
        // L'API indexe par préfixe nu : elle ne sait rien faire d'un OCG.
        // Mieux vaut un classeur japonais partiel qu'un classeur anglais.
        assert_eq!(
            decider(false, "LOCR-JP", Ok(&lignes_suspectes())),
            Decision::DepuisLocal
        );
    }

    #[test]
    fn un_classeur_ocg_absent_du_local_est_impossible() {
        let d = decider(false, "LOCR-JP", Err("absent".to_owned()));
        let Decision::Impossible(message) = d else {
            unreachable!("un OCG absent n'a aucune source de secours")
        };
        assert!(message.contains("LOCR-JP"));
        assert!(
            message.contains("MAJ BDD"),
            "le message doit dire quoi faire"
        );
    }

    #[test]
    fn une_liste_sans_carte_identifiee_n_est_pas_suspecte() {
        // Sans dénominateur, la moyenne n'a pas de sens : le Python ne conclut
        // pas plutôt que de conclure au hasard.
        let lignes = vec![LigneClasseur::default(); 3];
        let verdict = locales_semblent_incompletes(&lignes);
        assert!(!verdict.suspect);
        assert_eq!(verdict.cartes_uniques, 0);
        assert_eq!(verdict.moyenne, 0.0);
    }

    // ── L'orchestrateur ─────────────────────────────────────────────────────
    //
    // Tous ces tests sont HORS RÉSEAU : le chemin local ne demande rien, et
    // `avec_artworks` est faux. Le client HTTP est construit mais jamais
    // sollicité — le vérifier a demandé de choisir des cas où la décision est
    // `DepuisLocal`, ce qui est justement le chemin normal de l'application.

    /// Une installation avec sa `cardinfo.db` peuplée du set demandé.
    fn installation(n_fr: usize, n_en: usize) -> (tempfile::TempDir, Paths) {
        let dossier = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dossier.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        std::fs::create_dir_all(paths.classeurs()).unwrap();

        let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        let _ = conn.pragma_update(None, "foreign_keys", "OFF");
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CARDINFO).unwrap();
        set_bilingue(&conn, n_fr, n_en);
        drop(conn);
        (dossier, paths)
    }

    fn client() -> ygo_sources::ClientHttp {
        ygo_sources::ClientHttp::new().unwrap()
    }

    /// Exécute `creer` sans réseau ni passe artworks.
    fn creer_local(paths: &Paths, code: &str, greffons: &Greffons<'_>) -> Result<Issue> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(creer(paths, &client(), code, false, greffons))
    }

    fn compter(chemin: &Path) -> i64 {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(chemin).unwrap();
        conn.query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
            .unwrap()
    }

    #[test]
    fn le_chemin_local_ecrit_le_classeur_au_bon_endroit() {
        let (_d, paths) = installation(2, 2);
        let issue = creer_local(&paths, "RA05", &Greffons::default()).unwrap();

        assert!(matches!(
            issue,
            Issue::Cree {
                source: Source::Local,
                artworks: Artworks::NonDemandee,
                ..
            }
        ));
        let db = paths.classeur_db("RA05");
        assert!(db.is_file(), "la base doit exister");
        assert_eq!(compter(&db), 4, "deux cartes × deux raretés");
    }

    #[test]
    fn le_code_est_normalise_avant_toute_chose() {
        let (_d, paths) = installation(2, 2);
        creer_local(&paths, "  ra05  ", &Greffons::default()).unwrap();
        assert!(
            paths.classeur_db("RA05").is_file(),
            "le dossier porte le code en majuscules, espaces ôtés"
        );
    }

    #[test]
    fn creer_sur_un_classeur_peuple_ne_touche_a_rien() {
        let (_d, paths) = installation(2, 2);
        creer_local(&paths, "RA05", &Greffons::default()).unwrap();

        // Une marque que la recréation effacerait.
        let db = paths.classeur_db("RA05");
        let conn = ygo_db::connexion::ouvrir(&db).unwrap();
        conn.execute("UPDATE cards SET possessed = 1", []).unwrap();
        drop(conn);

        let issue = creer_local(&paths, "RA05", &Greffons::default()).unwrap();
        assert_eq!(issue, Issue::DejaExistant);

        let conn = ygo_db::connexion::ouvrir_lecture_seule(&db).unwrap();
        let possedees: i64 = conn
            .query_row("SELECT COUNT(*) FROM cards WHERE possessed = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(possedees, 4, "la possession de l'utilisateur est intacte");
    }

    /// Le bout-à-bout de la règle des fausses raretés, dans la forme exacte
    /// de CH01 : une carte, une illustration, deux raretés véritables et une
    /// mention d'artwork glissée par la source dans `set_rarity`.
    #[test]
    fn la_creation_ecarte_la_fausse_rarete_et_la_consigne() {
        let (_d, paths) = installation(0, 2);
        let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        conn.execute(
            "INSERT INTO set_prints
                (set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                 rarity, edition, qty, extended_art)
             SELECT set_uuid, id, 'C1', 'I1', 'RA05-EN001', 'New artwork',
                    'unlimited', 1, 0
               FROM set_locales WHERE set_uuid = 'S1' AND language = 'en'",
            (),
        )
        .unwrap();
        drop(conn);

        let issue = creer_local(&paths, "RA05", &Greffons::default()).unwrap();
        assert!(matches!(issue, Issue::Cree { lignes: 4, .. }), "{issue:?}");

        let db = paths.classeur_db("RA05");
        assert_eq!(compter(&db), 4, "la cinquième ligne n'entre pas en base");

        let ecarts = crate::classeur::ecarts_du_classeur(&db).unwrap();
        assert_eq!(ecarts.retirees(), 1);
        assert_eq!(ecarts.fantomes[0].rarete, "New artwork");
        assert_eq!(ecarts.fantomes[0].set_code, "RA05-EN001");
        assert!(
            ecarts.fantomes[0]
                .jumelles
                .contains(&"Ultra Rare".to_owned()),
            "le rapport nomme les raretés qui partageaient l'illustration"
        );
        assert!(!ecarts.quand.is_empty(), "et la date de la création");
        assert!(ecarts.resume().is_some(), "il y a une ligne à montrer");
    }

    /// Une création sans anomalie ne laisse rien derrière elle : pas de
    /// bandeau sur les classeurs qui vont bien, qui sont la règle.
    #[test]
    fn une_creation_sans_anomalie_ne_consigne_rien() {
        let (_d, paths) = installation(0, 2);
        creer_local(&paths, "RA05", &Greffons::default()).unwrap();
        assert_eq!(
            crate::classeur::ecarts_du_classeur(&paths.classeur_db("RA05")),
            None
        );
    }

    #[test]
    fn un_dossier_residuel_sans_base_est_purge_puis_recree() {
        // Le cas qui permet de retélécharger un classeur juste après l'avoir
        // supprimé : le dossier survit, la base non.
        let (_d, paths) = installation(2, 2);
        let dossier = paths.dossier_classeur("RA05");
        std::fs::create_dir_all(&dossier).unwrap();
        std::fs::write(dossier.join("RA05.db-wal"), b"orphelin").unwrap();

        let issue = creer_local(&paths, "RA05", &Greffons::default()).unwrap();
        assert!(matches!(issue, Issue::Cree { .. }));
        assert_eq!(compter(&paths.classeur_db("RA05")), 4);
        assert!(
            !dossier.join("RA05.db-wal").exists(),
            "le fichier WAL orphelin a disparu avec la purge"
        );
    }

    #[test]
    fn un_dossier_residuel_avec_une_base_vide_est_purge_aussi() {
        let (_d, paths) = installation(2, 2);
        let dossier = paths.dossier_classeur("RA05");
        std::fs::create_dir_all(&dossier).unwrap();
        let conn = ygo_db::connexion::ouvrir(paths.classeur_db("RA05")).unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        drop(conn);

        assert!(matches!(
            creer_local(&paths, "RA05", &Greffons::default()).unwrap(),
            Issue::Cree { .. }
        ));
        assert_eq!(compter(&paths.classeur_db("RA05")), 4);
    }

    #[test]
    fn un_set_ocg_absent_echoue_en_nommant_les_deux_sources() {
        // L'API YGOPRODeck n'indexe pas les sets par préfixe + langue : pour
        // un OCG absent en local, il n'y a aucun repli, et le message doit le
        // dire plutôt que de laisser l'utilisateur deviner.
        let (_d, paths) = installation(2, 2);
        let erreur = creer_local(&paths, "LOCH-JP", &Greffons::default()).unwrap_err();
        let texte = erreur.to_string();
        assert!(texte.contains("LOCH-JP"), "obtenu : {texte}");
        assert!(texte.contains("cardinfo.db"), "obtenu : {texte}");
        assert!(texte.contains("YGOPRODeck"), "obtenu : {texte}");
        assert!(texte.contains("MAJ BDD"), "et il dit quoi faire : {texte}");
    }

    #[test]
    fn le_greffon_de_rarete_agit_avant_l_ecriture() {
        // Le greffon pose une valeur QUE RIEN D'AUTRE ne produit, et le test
        // exige qu'elle soit sur toutes les lignes. Première version : il
        // remplaçait « Common » par « Ultra Rare » — or avec deux locales
        // complètes l'énumération choisit le FRANÇAIS, donc les raretés
        // écrites sont « Commune ». Le compte de « Common » valait zéro que le
        // greffon tourne ou non : le test ne prouvait rien.
        let (_d, paths) = installation(2, 2);
        let vu_code = std::cell::RefCell::new(String::new());
        let corriger = |code: &str, lignes: &mut Vec<LigneClasseur>| {
            vu_code.borrow_mut().push_str(code);
            for l in lignes.iter_mut() {
                l.rarity = "RARETÉ CORRIGÉE".to_owned();
            }
        };
        let greffons = Greffons {
            corriger_raretes: Some(&corriger),
            ..Greffons::default()
        };
        creer_local(&paths, "RA05", &greffons).unwrap();

        assert_eq!(
            vu_code.borrow().as_str(),
            "RA05",
            "le greffon reçoit le code normalisé"
        );
        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db("RA05")).unwrap();
        let (total, corrigees): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(*) FILTER (WHERE rarity = 'RARETÉ CORRIGÉE') FROM cards",
                [],
                |l| Ok((l.get(0)?, l.get(1)?)),
            )
            .unwrap();
        assert_eq!(total, 4);
        assert_eq!(corrigees, 4, "la correction est passée avant l'écriture");
    }

    /// Le rattrapage vit **après** la passe artworks, pas dans `creer`.
    ///
    /// Le premier correctif rattrapait les fantômes à la fin de `creer`. Or
    /// l'interface appelle `creer` avec `avec_artworks = false` puis lance la
    /// passe de son côté : le rattrapage passait avant le seul geste capable
    /// de démasquer un fantôme. Ce test fige les deux moitiés — `creer` seul
    /// ne touche à rien, le rattrapage d'après la passe tranche.
    #[test]
    fn le_rattrapage_attend_les_tirages_que_la_passe_artworks_ajoute() {
        let (_d, paths) = installation(2, 2);
        let corriger = |_code: &str, lignes: &mut Vec<LigneClasseur>| {
            for l in lignes.iter_mut() {
                if l.set_code.ends_with("002") {
                    l.rarity = "New".to_owned();
                }
            }
        };
        let greffons = Greffons {
            corriger_raretes: Some(&corriger),
            ..Greffons::default()
        };
        creer_local(&paths, "RA05", &greffons).unwrap();
        let chemin = paths.classeur_db("RA05");
        let raretes = ygo_core::rarity::Priorites::charger(paths.rarity_config());

        // Seule sur son illustration, la fausse rareté reste : c'est ce qu'on
        // demande à la règle, et `creer` ne la rejoue pas.
        assert_eq!(
            compter_raretes(&chemin, "New"),
            2,
            "sans tirage jumeau, rien à trancher"
        );
        assert_eq!(rattraper_fantomes(&chemin, "RA05", &raretes), 0);

        // Ce que fait la passe artworks : un tirage créé par héritage, donc
        // sur la même illustration.
        {
            let conn = ygo_db::connexion::ouvrir(&chemin).unwrap();
            let image: Option<i64> = conn
                .query_row(
                    "SELECT card_image_id FROM cards WHERE rarity = 'New' LIMIT 1",
                    [],
                    |l| l.get(0),
                )
                .unwrap();
            conn.execute(
                "INSERT INTO cards (set_code, rarity, card_image_id, name, quantite, possessed) \
                 VALUES ('RA05-FR002', 'Ultra Rare', ?1, 'Dragon Blanc', 0, 0)",
                [image],
            )
            .unwrap();
        }

        assert_eq!(
            rattraper_fantomes(&chemin, "RA05", &raretes),
            2,
            "entourée de vraies raretés, la fausse part"
        );
        assert_eq!(compter_raretes(&chemin, "New"), 0);
        assert_eq!(
            rattraper_fantomes(&chemin, "RA05", &raretes),
            0,
            "rejoué, le rattrapage ne trouve plus rien"
        );

        // Et il laisse une trace lisible dans le classeur.
        let ecarts = crate::classeur::ecarts_du_classeur(&chemin).unwrap();
        assert_eq!(ecarts.fantomes.len(), 2);
        assert!(
            ecarts.inconnues.is_empty(),
            "plus aucune « New » en base, plus aucun libellé inconnu annoncé : {:?}",
            ecarts.inconnues
        );
        assert!(
            ecarts.fantomes.iter().all(|f| !f.name.is_empty()),
            "le nom de la carte est repris de la base : {:?}",
            ecarts.fantomes
        );
    }

    fn compter_raretes(chemin: &Path, rarete: &str) -> i64 {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(chemin).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM cards WHERE rarity = ?1",
            [rarete],
            |l| l.get(0),
        )
        .unwrap()
    }

    /// La création pose l'image Yugipedia de chaque tirage, et une ligne dont
    /// le tirage n'a pas d'image garde la sienne.
    #[test]
    fn la_creation_pose_l_image_de_chaque_tirage() {
        let (_d, paths) = installation(2, 2);
        {
            let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
            conn.execute(
                "UPDATE set_prints SET edition = '1st', print_image_url = \
                   'https://ms.yugipedia.com//' || set_code || '-' || rarity || '.png' \
                  WHERE set_code = 'RA05-EN001'",
                [],
            )
            .unwrap();
        }
        creer_local(&paths, "RA05", &Greffons::default()).unwrap();

        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db("RA05")).unwrap();
        let mut req = conn
            .prepare("SELECT set_code, rarity, card_image_url FROM cards ORDER BY set_code, rarity")
            .unwrap();
        let lignes: Vec<(String, String, String)> = req
            .query_map([], |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (code, rarete, url) in &lignes {
            if code == "RA05-EN001" {
                assert_eq!(
                    url,
                    &format!("https://ms.yugipedia.com//{code}-{rarete}.png"),
                    "chaque rareté a l'image de son tirage"
                );
            } else {
                assert!(
                    !url.contains("yugipedia"),
                    "sans image de tirage, la ligne garde la sienne : {url}"
                );
            }
        }
        assert!(lignes.iter().any(|l| l.0 == "RA05-EN001"));
        assert!(lignes.iter().any(|l| l.0 != "RA05-EN001"));
    }

    #[test]
    fn le_greffon_d_overrides_recoit_le_chemin_de_la_base_ecrite() {
        let (_d, paths) = installation(2, 2);
        let vu = std::cell::RefCell::new(None);
        let overrides = |chemin: &Path| {
            *vu.borrow_mut() = Some(chemin.to_path_buf());
        };
        let greffons = Greffons {
            appliquer_overrides: Some(&overrides),
            ..Greffons::default()
        };
        creer_local(&paths, "RA05", &greffons).unwrap();
        assert_eq!(
            vu.borrow().as_deref(),
            Some(paths.classeur_db("RA05").as_path())
        );
    }

    #[test]
    fn sans_greffon_la_creation_se_fait_quand_meme() {
        // Ne rien brancher revient au chemin d'exception que le Python prévoit
        // déjà : Yugipedia injoignable, module d'anomalies absent.
        let (_d, paths) = installation(2, 2);
        let greffons = Greffons::default();
        assert!(greffons.corriger_raretes.is_none());
        assert!(matches!(
            creer_local(&paths, "RA05", &greffons).unwrap(),
            Issue::Cree { lignes: 4, .. }
        ));
    }

    #[test]
    fn le_debug_des_greffons_dit_lesquels_sont_branches() {
        let corriger = |_: &str, _: &mut Vec<LigneClasseur>| {};
        let greffons = Greffons {
            corriger_raretes: Some(&corriger),
            ..Greffons::default()
        };
        let texte = format!("{greffons:?}");
        assert!(texte.contains("corriger_raretes: true"), "obtenu : {texte}");
        assert!(
            texte.contains("appliquer_overrides: false"),
            "obtenu : {texte}"
        );
    }

    // ── L'arbitrage des sources, sans réseau ────────────────────────────────

    fn une_ligne(rarete: &str) -> Vec<LigneClasseur> {
        vec![LigneClasseur {
            rarity: rarete.to_owned(),
            ..ligne("RA05-EN001", "Ultra Rare")
        }]
    }

    #[test]
    fn l_arbitrage_ecrit_le_local_quand_il_suffit() {
        let issue = arbitrer("RA05", Decision::DepuisLocal, Ok(une_ligne("local")), None)
            .unwrap()
            .unwrap();
        assert_eq!(issue.1, Source::Local);
        assert_eq!(issue.0[0].rarity, "local");
    }

    #[test]
    fn l_arbitrage_prefere_l_api_quand_elle_repond() {
        let issue = arbitrer(
            "RA05",
            Decision::DepuisApi { repli_local: true },
            Ok(une_ligne("local")),
            Some(Ok(une_ligne("api"))),
        )
        .unwrap()
        .unwrap();
        assert_eq!(issue.1, Source::Api);
        assert_eq!(issue.0[0].rarity, "api", "l'API prime quand elle répond");
    }

    #[test]
    fn l_api_injoignable_retombe_sur_un_local_imparfait() {
        // C'est le compromis explicite du Python : « mieux qu'aucun classeur ».
        let issue = arbitrer(
            "RA05",
            Decision::DepuisApi { repli_local: true },
            Ok(une_ligne("local")),
            Some(Err("réseau coupé".to_owned())),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            issue.1,
            Source::LocalApresEchecApi,
            "la source dit que ce classeur est dégradé"
        );
        assert_eq!(issue.0[0].rarity, "local");
    }

    #[test]
    fn sans_repli_l_api_injoignable_est_fatale_et_nomme_les_deux_sources() {
        // Set absent en local : il n'y a rien sur quoi se rabattre.
        let erreur = arbitrer(
            "ZZZZ",
            Decision::DepuisApi { repli_local: false },
            Err("set « ZZZZ » introuvable".to_owned()),
            Some(Err("502 Bad Gateway".to_owned())),
        )
        .unwrap_err();
        let texte = erreur.to_string();
        assert!(texte.contains("ZZZZ"), "obtenu : {texte}");
        assert!(
            texte.contains("introuvable"),
            "ce que dit le local : {texte}"
        );
        assert!(texte.contains("502"), "ce que dit l'API : {texte}");
    }

    #[test]
    fn un_classeur_deja_existant_ne_donne_rien_a_ecrire() {
        assert!(
            arbitrer("RA05", Decision::DejaExistant, Ok(une_ligne("x")), None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn une_decision_impossible_remonte_son_message_tel_quel() {
        let erreur = arbitrer(
            "LOCH-JP",
            Decision::Impossible("message destiné à l'utilisateur".to_owned()),
            Err("absent".to_owned()),
            None,
        )
        .unwrap_err();
        assert_eq!(erreur.to_string(), "message destiné à l'utilisateur");
    }
}
