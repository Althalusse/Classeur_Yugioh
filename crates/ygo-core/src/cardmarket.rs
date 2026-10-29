// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Cardmarket — les règles pures qui mènent d'une carte du classeur à sa page.
//!
//! # Sans l'API, et sans jamais visiter le site
//!
//! Cardmarket publie chaque jour, sans compte, son catalogue et son guide des
//! prix (cf. `ygo_sources::cardmarket`). Ces fichiers donnent l'identifiant de
//! chaque produit (`idProduct`), son extension (`idExpansion`) et son nom
//! anglais — mais **ni sa rareté, ni son numéro**. Tout ce module sert à
//! combler ce trou : retrouver, pour un tirage `(set, carte, rareté)`, lequel
//! des produits de la carte est le bon.
//!
//! Le site lui-même est derrière Cloudflare : l'application n'y envoie aucune
//! requête. Elle **construit** des adresses, que l'utilisateur ouvre dans son
//! navigateur.
//!
//! # La règle d'ordre — établie sur pièces, octobre 2026
//!
//! Cardmarket numérote les versions d'une carte dans une extension :
//! « V.1 – Ultra Rare », « V.2 – Secret Rare »… Ce numéro suit **l'ordre des
//! raretés de l'extension**, pas l'ordre de création des produits (trois pages
//! le prouvent : BPRO, GS05, PHRE). L'ordre, relevé sur 30 sets confirmés par
//! l'utilisateur, sans une contradiction :
//!
//! 1. la **Common** d'abord ;
//! 2. les raretés **normales** en ordre croissant — Parallel, Rare, Super,
//!    Ultra, Gold, Premium Gold, Secret, Gold Secret ; une version Parallel
//!    juste après sa version normale ;
//! 3. les raretés **spéciales** — Ultimate, Collector's, Starlight, Prismatic,
//!    Quarter Century, Platinum, 20th, Extra Secret — dans un ordre **propre à
//!    chaque set** (SAST : 20th → Ultimate ; LIOV : Ultimate → Starlight ;
//!    BPRO : Starlight → Ultimate) ;
//! 4. la **Ghost** (« Holographic » en OCG) en dernier.
//!
//! Entre deux spéciales, l'ordre vient des sets confirmés
//! (`assets/cardmarket_ordres.json`) ou de quelques règles d'époque mesurées ;
//! à défaut, il reste inconnu et l'appelant se rabat sur la recherche.
//!
//! # Deux versions de même rareté — octobre 2026
//!
//! Une carte peut avoir plusieurs versions **de la même rareté** : RA02
//! « Rescue Cat » en a quatorze, sept raretés en deux illustrations ; les
//! cartes de légende de LOCH et LOCR ont une Ultra Rare au cadre ordinaire
//! et une Ultra Rare **Overframe**, dont l'illustration déborde du cadre.
//! Cardmarket les écrit pareil (« V.1 – Super Rare », « V.2 – Super Rare »).
//!
//! Le classeur, lui, les distingue par le fichier d'image de chaque ligne
//! (`…-RA02-EN-SR-1E-AA.png`, `…-LOCR-JP-UR-EA.png`) : c'est la
//! [`Distinction`]. Une version se nomme alors par une **clé** — la rareté,
//! et la distinction entre parenthèses ([`cle_de_version`]) : « Super Rare
//! (illustration 2) », « Ultra Rare (Overframe) ». Les pages ne les rangent
//! pas toutes de la même façon :
//!
//! - RA02 (« Rescue Cat ») et RA04 (« El Shaddoll Winda ») mettent
//!   l'alternative **juste après** l'originale de sa rareté : la règle vaut
//!   alors pour tout le set ([`illustration_suit_son_originale`]) ;
//! - RA05 (« Raigeki ») met ses alternatives **en tête**, avant toutes les
//!   autres versions : seules les versions relevées y sont placées ;
//! - l'**Overframe** ne suit pas sa rareté : LOCH et LOCR la rangent après la
//!   Secret (UR, ScR, UR Overframe, Prismatic, Grand Master). Seul un ordre
//!   relevé la place ;
//! - sans page, rien ne place une alternative.
//!
//! Et l'identifiant d'une alternative ne se déduit pas toujours de l'ordre de
//! création : RA04 a créé celle de « El Shaddoll Winda » quatre jours avant
//! les autres versions ([`illustrations_saisies_a_part`]).
//!
//! # Une hypothèse retirée
//!
//! « En OCG ancien, l'Ultimate passe avant la Secret » était un artefact des
//! prix : elle avait été déduite en supposant l'Ultimate plus chère, alors
//! qu'en OCG c'est la Secret (Firewall Dragon : 5,95 € contre 3,99 €). Les
//! pages COTD, SAST, LIOV et DAMA l'ont démentie. D'où la règle : **le modèle
//! décide, les prix ne servent qu'à vérifier le sens de saisie** ([`Sens`]).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// L'adresse du site.
pub const SITE: &str = "https://www.cardmarket.com";

/// Les ordres confirmés et les libellés de Cardmarket, compilés dans le binaire.
const TABLE_JSON: &str = include_str!("../../../assets/cardmarket_ordres.json");

const COMMON: &str = "Common";
const SUPER: &str = "Super Rare";
const ULTRA: &str = "Ultra Rare";
const SECRET: &str = "Secret Rare";
const ULTIMATE: &str = "Ultimate Rare";
const COLLECTOR: &str = "Collector's Rare";
const STARLIGHT: &str = "Starlight Rare";
const PRISMATIC: &str = "Prismatic Secret Rare";
const QUARTER_CENTURY: &str = "Quarter Century Secret Rare";
const VINGTIEME: &str = "20th Secret Rare";
const GHOST: &str = "Ghost Rare";

/// Les deux mondes du jeu, que Cardmarket range dans des extensions séparées.
///
/// Une extension TCG couvre toutes les langues occidentales (un `RA04-FR001`
/// et un `RA04-EN001` sont le même produit) ; l'OCG japonais a les siennes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    /// Cartes occidentales — anglais, français, allemand…
    Tcg,
    /// Cartes japonaises.
    Ocg,
}

impl Region {
    /// La région d'une langue de `set_locales`, ou `None` pour celles que
    /// Cardmarket range à part (coréen, chinois, anglais asiatique) et que le
    /// rapprochement ne couvre pas.
    ///
    /// ```
    /// use ygo_core::cardmarket::Region;
    /// assert_eq!(Region::de_langue("fr"), Some(Region::Tcg));
    /// assert_eq!(Region::de_langue("na"), Some(Region::Tcg));
    /// assert_eq!(Region::de_langue("jp"), Some(Region::Ocg));
    /// assert_eq!(Region::de_langue("kr"), None);
    /// ```
    #[must_use]
    pub fn de_langue(langue: &str) -> Option<Self> {
        match langue.trim().to_ascii_lowercase().as_str() {
            "jp" | "ja" => Some(Self::Ocg),
            "kr" | "ko" | "ae" | "sc" | "tc" | "zh" | "zh-cn" | "zh-tw" => None,
            "" => None,
            _ => Some(Self::Tcg),
        }
    }
}

/// Le nom d'une carte réduit à ce qui la distingue : minuscules ASCII et
/// chiffres, accents repliés.
///
/// Cardmarket écrit `""A"" Cell Breeding Device` et parfois `… (card)` ; la
/// base, `"A" Cell Breeding Device`. Les deux doivent se rejoindre.
///
/// ```
/// use ygo_core::cardmarket::normaliser_nom;
/// assert_eq!(normaliser_nom("\"\"A\"\" Cell Breeding Device"), "acellbreedingdevice");
/// assert_eq!(normaliser_nom("\"A\" Cell Breeding Device"), "acellbreedingdevice");
/// assert_eq!(normaliser_nom("Polymerization (card)"), "polymerization");
/// assert_eq!(normaliser_nom("Gaïa the Fierce Knight"), "gaiathefierceknight");
/// assert_eq!(normaliser_nom("Dark Magician, the Pharaoh's Servant"), "darkmagicianthepharaohsservant");
/// ```
#[must_use]
pub fn normaliser_nom(nom: &str) -> String {
    let sans_doubles = nom.replace("\"\"", "\"");
    let mut base = sans_doubles.trim_end();
    if let Some(reste) = base.strip_suffix("(card)") {
        base = reste.trim_end();
    }
    let mut sortie = String::with_capacity(base.len());
    for c in base.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            sortie.push(c);
        } else if let Some(r) = replier(c) {
            sortie.push_str(r);
        }
    }
    sortie
}

/// L'équivalent ASCII d'une lettre accentuée, d'un chiffre ou d'une lettre
/// pleine chasse ; `None` pour tout le reste, qui est ignoré.
fn replier(c: char) -> Option<&'static str> {
    const CHIFFRES: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const LETTRES: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    let r = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        '０'..='９' => {
            let rang = (c as u32).checked_sub('０' as u32)?;
            return CHIFFRES.get(usize::try_from(rang).ok()?).copied();
        }
        'ａ'..='ｚ' => {
            let rang = (c as u32).checked_sub('ａ' as u32)?;
            return LETTRES.get(usize::try_from(rang).ok()?).copied();
        }
        _ => return None,
    };
    Some(r)
}

// ─────────────────────────────────────────────────────────────────────────────
// Ordre des versions
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'il faut savoir d'une extension pour en ordonner les raretés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContexteOrdre<'a> {
    /// L'extension Cardmarket — clé des ordres confirmés.
    pub id_extension: i64,
    /// TCG ou OCG.
    pub region: Region,
    /// Date de sortie du set (`AAAA-MM-JJ`), pour les règles d'époque.
    pub sortie: Option<&'a str>,
}

/// Le rang d'une rareté **normale** ; `None` pour une spéciale ou une
/// rareté que la règle ne connaît pas.
fn rang_normal(rarete: &str) -> Option<u8> {
    Some(match rarete {
        COMMON => 0,
        "Short Print" | "Super Short Print" => 5,
        "Common Parallel Rare" => 10,
        "Millennium Rare" => 15,
        "Rare" => 20,
        SUPER => 30,
        "Super Parallel Rare" => 35,
        ULTRA => 40,
        "Gold Rare" => 43,
        // Pas d'« Ultra Parallel Rare » ici : face à la Secret, elle passe
        // avant dans 20TH-JPC (où Cardmarket l'appelle « Ultra Rare »). Seule
        // sa place après l'Ultra est une règle. (Ce que LOCH affiche « Ultra
        // Parallel Rare » après la Secret est l'Ultra Overframe des cartes de
        // légende : une autre version, cf. `Distinction::Overframe`.)
        "Millennium Ultra Rare" => 46,
        "Premium Gold Rare" => 50,
        SECRET => 60,
        "Gold Secret Rare" => 63,
        "Secret Parallel Rare" => 65,
        _ => return None,
    })
}

/// Les raretés **spéciales** : elles viennent après toutes les normales.
fn est_speciale(rarete: &str) -> bool {
    matches!(
        rarete,
        ULTIMATE
            | COLLECTOR
            | STARLIGHT
            | PRISMATIC
            | QUARTER_CENTURY
            | VINGTIEME
            | GHOST
            | "Platinum Secret Rare"
            | "Extra Secret Rare"
            | "Grand Master Rare"
            | "Ghost/Gold Rare"
            | "10000 Secret Rare"
    )
}

/// La version normale dont une rareté est le parallèle, s'il y a lieu.
fn normale_de(parallele: &str) -> Option<&'static str> {
    Some(match parallele {
        "Common Parallel Rare"
        | "Duel Terminal Common Parallel Rare"
        | "Duel Terminal Normal Parallel Rare" => COMMON,
        "Rare Parallel Rare" | "Duel Terminal Rare Parallel Rare" => "Rare",
        "Super Parallel Rare" | "Duel Terminal Super Parallel Rare" => SUPER,
        "Ultra Parallel Rare" | "Duel Terminal Ultra Parallel Rare" => ULTRA,
        "Secret Parallel Rare" | "Duel Terminal Secret Parallel Rare" => SECRET,
        "Extra Secret Parallel Rare" => "Extra Secret Rare",
        _ => return None,
    })
}

/// La paire `{a, b}` est-elle exactement `{x, y}` ?
fn paire(a: &str, b: &str, x: &str, y: &str) -> bool {
    (a == x && b == y) || (a == y && b == x)
}

/// `a` passe-t-il avant `b` dans les versions de l'extension ?
///
/// `a` et `b` sont des raretés, ou des clés de version ([`cle_de_version`]).
/// `Some(true)` : avant ; `Some(false)` : après ; `None` : on ne sait pas.
/// Les ordres confirmés de l'extension priment ; la règle générale ensuite.
///
/// ```
/// use ygo_core::cardmarket::{avant, ContexteOrdre, Region};
/// let tcg = ContexteOrdre { id_extension: 0, region: Region::Tcg, sortie: None };
/// assert_eq!(avant("Common", "Secret Rare", &tcg), Some(true));
/// assert_eq!(avant("Ultimate Rare", "Ultra Rare", &tcg), Some(false));
/// assert_eq!(avant("Secret Rare", "Starlight Rare", &tcg), Some(true));
/// assert_eq!(avant("Ghost Rare", "Ultimate Rare", &tcg), Some(false));
/// // Deux spéciales sans règle : on ne sait pas.
/// assert_eq!(avant("Ultimate Rare", "Platinum Secret Rare", &tcg), None);
/// // Une illustration alternative : sans page du set, rien ne la place…
/// assert_eq!(avant("Super Rare (illustration 2)", "Ultra Rare", &tcg), None);
/// assert_eq!(avant("Super Rare", "Super Rare (illustration 2)", &tcg), None);
/// // … RA04 (« El Shaddoll Winda ») la montre juste après son originale,
/// // et la règle vaut pour tout le set.
/// let ra04 = ContexteOrdre { id_extension: 5989, ..tcg };
/// let (pl, pl2) = ("Platinum Secret Rare", "Platinum Secret Rare (illustration 2)");
/// assert_eq!(avant(pl, pl2, &ra04), Some(true));
/// assert_eq!(avant(pl2, "Quarter Century Secret Rare", &ra04), Some(true));
/// // RA05 (« Raigeki ») range les siennes en tête.
/// let ra05 = ContexteOrdre { id_extension: 6424, ..tcg };
/// assert_eq!(avant("Ultra Rare (illustration 2)", "Super Rare", &ra05), Some(true));
/// ```
#[must_use]
pub fn avant(a: &str, b: &str, ctx: &ContexteOrdre<'_>) -> Option<bool> {
    if a == b {
        return None;
    }
    for sequence in ordres_confirmes(ctx.id_extension) {
        let i = sequence.iter().position(|r| r == a);
        let j = sequence.iter().position(|r| r == b);
        if let (Some(i), Some(j)) = (i, j) {
            return Some(i < j);
        }
    }
    let (rarete_a, da) = decouper_cle(a);
    let (rarete_b, db) = decouper_cle(b);
    if da == Distinction::Aucune && db == Distinction::Aucune {
        return modele(a, b, ctx);
    }
    // L'Overframe : les pages seules la placent (LOCH, LOCR).
    if da == Distinction::Overframe || db == Distinction::Overframe {
        return None;
    }
    // Une illustration alternative : seulement dans un set dont une page la
    // montre juste après son originale (RA02, RA04). Ailleurs, rien ne la
    // place — RA05 range les siennes en tête, avant toutes les autres.
    if !illustration_suit_son_originale(ctx.id_extension) {
        return None;
    }
    if rarete_a == rarete_b {
        return match (da, db) {
            (Distinction::Aucune, Distinction::Illustration(_)) => Some(true),
            (Distinction::Illustration(_), Distinction::Aucune) => Some(false),
            // Deux alternatives entre elles : aucune page ne les a départagées.
            _ => None,
        };
    }
    // Elle suit son originale : face à une autre rareté, elle se range comme elle.
    avant(rarete_a, rarete_b, ctx)
}

/// La règle générale, sans les ordres confirmés.
fn modele(a: &str, b: &str, ctx: &ContexteOrdre<'_>) -> Option<bool> {
    if a == COMMON || b == COMMON {
        return Some(a == COMMON);
    }
    if normale_de(b) == Some(a) {
        return Some(true);
    }
    if normale_de(a) == Some(b) {
        return Some(false);
    }
    match (rang_normal(a), rang_normal(b)) {
        (Some(x), Some(y)) if x != y => return Some(x < y),
        (Some(_), None) if est_speciale(b) => return Some(true),
        (None, Some(_)) if est_speciale(a) => return Some(false),
        _ => {}
    }
    if !(est_speciale(a) && est_speciale(b)) {
        return None;
    }
    if a == GHOST || b == GHOST {
        return Some(b == GHOST);
    }
    let sortie = ctx.sortie.unwrap_or("");
    match ctx.region {
        Region::Ocg => {
            // SAST confirmé, et cinq boosters 2018–2020 d'après les prix.
            if paire(a, b, VINGTIEME, ULTIMATE) {
                return Some(a == VINGTIEME);
            }
            // LIOV et DAMA confirmés, ROTD, PHRA et BLVO d'après les prix.
            if paire(a, b, ULTIMATE, PRISMATIC) && ("2020-01"..="2022-05-99").contains(&sortie) {
                return Some(a == ULTIMATE);
            }
            // RC04, DUNE, AGOV : Ultimate puis QC ; PHNI à ALIN : l'inverse.
            if paire(a, b, ULTIMATE, QUARTER_CENTURY) {
                if ("2023-02"..="2023-08-99").contains(&sortie) {
                    return Some(a == ULTIMATE);
                }
                if ("2023-09"..="2025-02-99").contains(&sortie) {
                    return Some(a == QUARTER_CENTURY);
                }
            }
        }
        Region::Tcg => {
            // VASM, PHRE confirmés ; CRBR, JUSH, MZTM d'après les prix. Les
            // Rarity Collections, qui font l'inverse, sont confirmées.
            if a == COLLECTOR && (b == STARLIGHT || b == QUARTER_CENTURY) {
                return Some(true);
            }
            if b == COLLECTOR && (a == STARLIGHT || a == QUARTER_CENTURY) {
                return Some(false);
            }
        }
    }
    None
}

/// Les raretés d'une carte dans l'ordre des versions Cardmarket, si cet
/// ordre est **entièrement** déterminé.
///
/// Une paire inconnue peut se déduire des autres (a < b et b < c donnent
/// a < c) ; c'est pourquoi on ferme la relation avant de conclure. Une
/// contradiction, ou une paire que rien ne tranche, rend `None`.
///
/// ```
/// use ygo_core::cardmarket::{ordonner, ContexteOrdre, Region};
/// let ctx = ContexteOrdre { id_extension: 0, region: Region::Ocg, sortie: Some("2018-10-13") };
/// let r = ["Ghost Rare", "Ultimate Rare", "Secret Rare", "20th Secret Rare", "Ultra Rare"]
///     .map(String::from);
/// assert_eq!(
///     ordonner(&r, &ctx).unwrap(),
///     ["Ultra Rare", "Secret Rare", "20th Secret Rare", "Ultimate Rare", "Ghost Rare"]
/// );
/// ```
#[must_use]
pub fn ordonner(raretes: &[String], ctx: &ContexteOrdre<'_>) -> Option<Vec<String>> {
    let mut uniques: Vec<&String> = Vec::new();
    for r in raretes {
        if !uniques.contains(&r) {
            uniques.push(r);
        }
    }
    let n = uniques.len();
    let mut avant_que: HashSet<(usize, usize)> = HashSet::new();
    for (i, a) in uniques.iter().enumerate() {
        for (j, b) in uniques.iter().enumerate().skip(i + 1) {
            match avant(a, b, ctx) {
                Some(true) => {
                    avant_que.insert((i, j));
                }
                Some(false) => {
                    avant_que.insert((j, i));
                }
                None => {}
            }
        }
    }
    // Fermeture transitive — n ne dépasse jamais une dizaine.
    loop {
        let mut ajouts = Vec::new();
        for &(i, j) in &avant_que {
            for &(k, l) in &avant_que {
                if j == k && !avant_que.contains(&(i, l)) {
                    ajouts.push((i, l));
                }
            }
        }
        if ajouts.is_empty() {
            break;
        }
        avant_que.extend(ajouts);
    }
    let mut rangs: Vec<(usize, &String)> = Vec::with_capacity(n);
    for (i, r) in uniques.iter().enumerate() {
        if avant_que.contains(&(i, i)) {
            return None; // contradiction
        }
        let precedents = (0..n).filter(|&j| avant_que.contains(&(j, i))).count();
        rangs.push((precedents, r));
    }
    rangs.sort_by_key(|(p, _)| *p);
    // Ordre total : les rangs sont exactement 0, 1, …, n−1.
    if rangs
        .iter()
        .enumerate()
        .any(|(attendu, (p, _))| *p != attendu)
    {
        return None;
    }
    Some(rangs.into_iter().map(|(_, r)| r.clone()).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Versions de même rareté
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qui sépare deux versions **de même rareté** d'une carte dans un set.
///
/// Le classeur le dit par le fichier d'image de la ligne ; Cardmarket, par
/// l'image seule — ses libellés sont identiques.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Distinction {
    /// L'illustration d'origine, au cadre ordinaire.
    #[default]
    Aucune,
    /// Une autre illustration : 2 pour la deuxième, 3 pour la troisième…
    Illustration(u32),
    /// L'Overframe : l'illustration déborde du cadre (Yugipedia : `-EA`).
    Overframe,
}

impl Distinction {
    /// La distinction que désigne le suffixe d'un fichier Yugipedia — ce que
    /// `ygo_sources::yugipedia::artwork::suffixe` en extrait.
    ///
    /// Rien → l'originale ; `EA` → l'Overframe ; `AA` → la deuxième
    /// illustration, `AA2` la troisième (RA04 « Dark Magician » en a six) ;
    /// une lettre de `A` à `H` ou un chiffre de 1 à 9 → ce rang (LCKC,
    /// LDK2 ; `A` et `1` sont l'originale). Tout autre suffixe — `J` et `K`
    /// désignent les decks de Joey et de Kaiba, pas des illustrations — rend
    /// `None`.
    ///
    /// ```
    /// use ygo_core::cardmarket::Distinction;
    /// assert_eq!(Distinction::du_suffixe(""), Some(Distinction::Aucune));
    /// assert_eq!(Distinction::du_suffixe("AA"), Some(Distinction::Illustration(2)));
    /// assert_eq!(Distinction::du_suffixe("AA2"), Some(Distinction::Illustration(3)));
    /// assert_eq!(Distinction::du_suffixe("C"), Some(Distinction::Illustration(3)));
    /// assert_eq!(Distinction::du_suffixe("2"), Some(Distinction::Illustration(2)));
    /// assert_eq!(Distinction::du_suffixe("EA"), Some(Distinction::Overframe));
    /// assert_eq!(Distinction::du_suffixe("J"), None);
    /// ```
    #[must_use]
    pub fn du_suffixe(suffixe: &str) -> Option<Self> {
        let s = suffixe.trim().to_ascii_uppercase();
        let rang = |n: u32| {
            if n <= 1 {
                Self::Aucune
            } else {
                Self::Illustration(n)
            }
        };
        if s.is_empty() {
            return Some(Self::Aucune);
        }
        if s == "EA" {
            return Some(Self::Overframe);
        }
        if let Some(chiffres) = s.strip_prefix("AA") {
            if chiffres.is_empty() {
                return Some(Self::Illustration(2));
            }
            let n: u32 = chiffres.parse().ok()?;
            if n == 0 {
                return None;
            }
            return n.checked_add(1).map(Self::Illustration);
        }
        match s.as_bytes() {
            [lettre @ b'A'..=b'H'] => Some(rang(u32::from(lettre - b'A') + 1)),
            [chiffre @ b'1'..=b'9'] => Some(rang(u32::from(chiffre - b'0'))),
            _ => None,
        }
    }

    /// La précision montrée après le libellé de Cardmarket, s'il en faut une.
    #[must_use]
    pub fn precision(self) -> Option<String> {
        match self {
            Self::Aucune => None,
            Self::Illustration(n) => Some(format!("illustration {n}")),
            Self::Overframe => Some("Overframe".to_owned()),
        }
    }
}

/// Préfixe de la précision « illustration ».
const ILLUSTRATION: &str = "illustration ";

/// La clé d'une version : sa rareté, et ce qui la distingue de ses sœurs de
/// même rareté entre parenthèses.
///
/// ```
/// use ygo_core::cardmarket::{cle_de_version, Distinction};
/// assert_eq!(cle_de_version("Super Rare", Distinction::Aucune), "Super Rare");
/// assert_eq!(
///     cle_de_version("Super Rare", Distinction::Illustration(2)),
///     "Super Rare (illustration 2)"
/// );
/// assert_eq!(cle_de_version("Ultra Rare", Distinction::Overframe), "Ultra Rare (Overframe)");
/// ```
#[must_use]
pub fn cle_de_version(rarete: &str, distinction: Distinction) -> String {
    match distinction.precision() {
        Some(p) => format!("{rarete} ({p})"),
        None => rarete.to_owned(),
    }
}

/// L'inverse de [`cle_de_version`] : la rareté et la distinction d'une clé.
/// Une rareté seule rend [`Distinction::Aucune`].
///
/// ```
/// use ygo_core::cardmarket::{decouper_cle, Distinction};
/// assert_eq!(decouper_cle("Super Rare (illustration 2)"), ("Super Rare", Distinction::Illustration(2)));
/// assert_eq!(decouper_cle("Ultra Rare (Overframe)"), ("Ultra Rare", Distinction::Overframe));
/// assert_eq!(decouper_cle("Ghost/Gold Rare"), ("Ghost/Gold Rare", Distinction::Aucune));
/// ```
#[must_use]
pub fn decouper_cle(cle: &str) -> (&str, Distinction) {
    let Some((rarete, reste)) = cle.rsplit_once(" (") else {
        return (cle, Distinction::Aucune);
    };
    let Some(precision) = reste.strip_suffix(')') else {
        return (cle, Distinction::Aucune);
    };
    if precision == "Overframe" {
        return (rarete, Distinction::Overframe);
    }
    match precision
        .strip_prefix(ILLUSTRATION)
        .and_then(|n| n.parse::<u32>().ok())
    {
        Some(n) if n >= 2 => (rarete, Distinction::Illustration(n)),
        _ => (cle, Distinction::Aucune),
    }
}

/// Une clé de version dans le vocabulaire de ce module : la rareté à double
/// nom y prend son nom TCG ([`crate::rarity::region`]).
///
/// Les ordres relevés, les règles et la table des libellés parlent de la
/// `Ghost Rare` ; un classeur japonais écrit `Holographic Rare` depuis le
/// 2026-10-26. [`libelle`] rend ensuite « Holographic Rare » pour l'OCG,
/// comme Cardmarket.
///
/// ```
/// use ygo_core::cardmarket::cle_de_reference;
/// assert_eq!(cle_de_reference("Holographic Rare"), "Ghost Rare");
/// assert_eq!(
///     cle_de_reference("Holographic Rare (illustration 2)"),
///     "Ghost Rare (illustration 2)"
/// );
/// assert_eq!(cle_de_reference("Ultra Rare (Overframe)"), "Ultra Rare (Overframe)");
/// ```
#[must_use]
pub fn cle_de_reference(cle: &str) -> String {
    let (rarete, distinction) = decouper_cle(cle);
    let reference = crate::rarity::region::nom_de_reference(rarete);
    if reference == rarete {
        cle.to_owned()
    } else {
        cle_de_version(reference, distinction)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Table des ordres confirmés et des libellés
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct Table {
    #[serde(default)]
    ordres: HashMap<String, Vec<Vec<String>>>,
    /// Par extension, ce qui montre que l'ordre de création des produits ne
    /// dit pas lequel est l'illustration alternative.
    #[serde(default)]
    illustrations_saisies_a_part: HashMap<String, String>,
    #[serde(default)]
    libelles: Libelles,
}

#[derive(Debug, Default, Deserialize)]
struct Libelles {
    #[serde(default)]
    defaut: HashMap<String, String>,
    #[serde(default)]
    ocg: HashMap<String, String>,
    #[serde(default)]
    par_extension: HashMap<String, HashMap<String, String>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(TABLE_JSON).unwrap_or_default())
}

/// Les ordres relevés sur les pages de cette extension — vide si aucun.
#[must_use]
pub fn ordres_confirmes(id_extension: i64) -> &'static [Vec<String>] {
    table()
        .ordres
        .get(&id_extension.to_string())
        .map_or(&[], Vec::as_slice)
}

/// Une page relevée de ce set montre-t-elle une illustration alternative
/// **juste après** son originale ? La règle vaut alors pour tout le set :
/// toute alternative suit l'originale de sa rareté.
///
/// RA02 (« Rescue Cat », sept raretés) et RA04 (« El Shaddoll Winda »,
/// Quarter Century) le montrent ; RA05 (« Raigeki ») non — il range ses
/// alternatives en tête, avant toutes les autres versions.
///
/// ```
/// use ygo_core::cardmarket::illustration_suit_son_originale;
/// assert!(illustration_suit_son_originale(5604)); // RA02
/// assert!(illustration_suit_son_originale(5989)); // RA04
/// assert!(!illustration_suit_son_originale(6424)); // RA05
/// assert!(!illustration_suit_son_originale(5848)); // RA03 : aucune page
/// ```
#[must_use]
pub fn illustration_suit_son_originale(id_extension: i64) -> bool {
    ordres_confirmes(id_extension).iter().any(|sequence| {
        sequence.windows(2).any(|paire| match paire {
            [a, b] => {
                let (rarete_a, da) = decouper_cle(a);
                let (rarete_b, db) = decouper_cle(b);
                rarete_a == rarete_b
                    && da == Distinction::Aucune
                    && matches!(db, Distinction::Illustration(_))
            }
            _ => false,
        })
    })
}

/// Dans ce set, l'ordre de création des produits ne dit pas lequel est
/// l'illustration alternative : le rang V.n d'une version reste sûr, son
/// identifiant non.
///
/// RA04 : l'alternative Quarter Century de « El Shaddoll Winda » (V.6) a été
/// créée quatre jours avant les sept autres versions.
///
/// ```
/// use ygo_core::cardmarket::illustrations_saisies_a_part;
/// assert!(illustrations_saisies_a_part(5989)); // RA04
/// assert!(!illustrations_saisies_a_part(5604)); // RA02
/// ```
#[must_use]
pub fn illustrations_saisies_a_part(id_extension: i64) -> bool {
    table()
        .illustrations_saisies_a_part
        .contains_key(&id_extension.to_string())
}

/// Les extensions dont un ordre a été confirmé.
#[must_use]
pub fn extensions_confirmees() -> Vec<i64> {
    let mut ids: Vec<i64> = table()
        .ordres
        .keys()
        .filter_map(|k| k.parse().ok())
        .collect();
    ids.sort_unstable();
    ids
}

/// Comment Cardmarket écrit cette rareté — ou cette version, pour une clé de
/// [`cle_de_version`] — dans cette extension.
///
/// Une clé que la table ne nomme pas s'écrit comme sa rareté : Cardmarket
/// ne distingue deux illustrations que par l'image.
///
/// ```
/// use ygo_core::cardmarket::{libelle, Region};
/// assert_eq!(libelle("Ghost Rare", Region::Ocg, 0), "Holographic Rare");
/// assert_eq!(libelle("Ghost Rare", Region::Tcg, 0), "Ghost Rare");
/// assert_eq!(libelle("Collector's Rare", Region::Tcg, 0), "Collectors Rare");
/// // LOCH écrit « Secret Parallel » ce que d'autres sets OCG écrivent « Starlight ».
/// assert_eq!(libelle("Prismatic Secret Rare", Region::Ocg, 6508), "Secret Parallel Rare");
/// assert_eq!(libelle("Prismatic Secret Rare", Region::Ocg, 4540), "Starlight Rare");
/// // L'Ultra Overframe des légendes : « Ultra Parallel » dans LOCH, « Ultra » dans LOCR.
/// assert_eq!(libelle("Ultra Rare (Overframe)", Region::Ocg, 6508), "Ultra Parallel Rare");
/// assert_eq!(libelle("Ultra Rare (Overframe)", Region::Ocg, 6512), "Ultra Rare");
/// assert_eq!(libelle("Collector's Rare (illustration 2)", Region::Tcg, 5604), "Collectors Rare");
/// ```
#[must_use]
pub fn libelle(cle: &str, region: Region, id_extension: i64) -> String {
    let l = &table().libelles;
    let propres = l.par_extension.get(&id_extension.to_string());
    if let Some(propre) = propres.and_then(|m| m.get(cle)) {
        return propre.clone();
    }
    let (rarete, _) = decouper_cle(cle);
    if let Some(propre) = propres.and_then(|m| m.get(rarete)) {
        return propre.clone();
    }
    if region == Region::Ocg {
        if let Some(ocg) = l.ocg.get(rarete) {
            return ocg.clone();
        }
    }
    l.defaut
        .get(rarete)
        .cloned()
        .unwrap_or_else(|| rarete.to_owned())
}

/// Le libellé à montrer pour une version : celui de Cardmarket, suivi de la
/// précision qui la sépare de ses sœurs de même rareté — sur la page, seule
/// l'image les distingue.
///
/// ```
/// use ygo_core::cardmarket::{libelle_de_version, Region};
/// assert_eq!(libelle_de_version("Secret Rare", Region::Tcg, 0), "Secret Rare");
/// assert_eq!(
///     libelle_de_version("Super Rare (illustration 2)", Region::Tcg, 5604),
///     "Super Rare (illustration 2)"
/// );
/// assert_eq!(
///     libelle_de_version("Ultra Rare (Overframe)", Region::Ocg, 6508),
///     "Ultra Parallel Rare (Overframe)"
/// );
/// ```
#[must_use]
pub fn libelle_de_version(cle: &str, region: Region, id_extension: i64) -> String {
    let base = libelle(cle, region, id_extension);
    match decouper_cle(cle).1.precision() {
        Some(p) => format!("{base} ({p})"),
        None => base,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sens de saisie des produits
// ─────────────────────────────────────────────────────────────────────────────

/// La valeur relative d'une rareté — ce qu'on attend d'elle en prix.
///
/// Elle ne sert **qu'à vérifier** le sens de saisie d'une extension, sur des
/// paires très écartées ([`ECART_DE_VALEUR`]) : entre une Common et une
/// Secret, le prix ne se trompe pas ; entre une Secret et une Ultimate OCG, si.
///
/// Une clé de version vaut sa rareté — sauf l'Overframe, dont on ne sait rien
/// d'avance (LOCR « Blue-Eyes White Dragon, the White Phantom Beast » :
/// 0,48 € l'Ultra ordinaire, 35 € l'Overframe) : elle ne témoigne pas.
///
/// ```
/// use ygo_core::cardmarket::valeur;
/// assert_eq!(valeur("Secret Rare"), Some(4.0));
/// assert_eq!(valeur("Secret Rare (illustration 2)"), Some(4.0));
/// assert_eq!(valeur("Ultra Rare (Overframe)"), None);
/// ```
#[must_use]
pub fn valeur(cle: &str) -> Option<f64> {
    match decouper_cle(cle) {
        (_, Distinction::Overframe) => None,
        (rarete, _) => valeur_de_rarete(rarete),
    }
}

/// La table de [`valeur`].
fn valeur_de_rarete(rarete: &str) -> Option<f64> {
    Some(match rarete {
        COMMON => 0.0,
        "Short Print" => 0.3,
        "Super Short Print" => 0.5,
        "Rare" => 1.0,
        "Starfoil Rare" | "Shatterfoil Rare" | "Mosaic Rare" => 1.1,
        "Common Parallel Rare" | "Duel Terminal Common Parallel Rare" => 1.2,
        "Millennium Rare" => 1.5,
        SUPER => 2.0,
        "Super Parallel Rare" | ULTRA | "Gold Rare" => 3.0,
        "Premium Gold Rare" => 3.5,
        "Ultra Parallel Rare" | SECRET | "Platinum Rare" => 4.0,
        "Gold Secret Rare" => 4.5,
        COLLECTOR => 4.6,
        ULTIMATE => 4.7,
        "Secret Parallel Rare" => 5.0,
        "Extra Secret Rare" | "Platinum Secret Rare" => 5.5,
        PRISMATIC | VINGTIEME => 6.0,
        GHOST | QUARTER_CENTURY => 7.0,
        STARLIGHT => 8.0,
        _ => return None,
    })
}

/// Écart de valeur minimal pour qu'une paire témoigne du sens de saisie.
pub const ECART_DE_VALEUR: f64 = 2.0;

/// Nombre de témoins en deçà duquel on ne conclut pas.
pub const TEMOINS_MINIMUM: u32 = 4;

/// Dans quel ordre Cardmarket a **créé** les produits d'une extension.
///
/// Le numéro V.n suit l'ordre des raretés ; l'`idProduct`, l'ordre de
/// création. Les deux coïncident le plus souvent — pas toujours : GS05 a été
/// saisi à l'envers une carte sur deux, BLMM entièrement à l'envers. C'est ce
/// qui décide si l'on peut attribuer un `idProduct` (donc un prix) à une
/// version sans voir la page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Sens {
    /// Créés dans l'ordre des versions : le n-ième identifiant est la V.n.
    DansLOrdre,
    /// Créés à rebours : le dernier identifiant est la V.1.
    Inverse,
    /// Ni l'un ni l'autre : on ne peut pas attribuer les identifiants.
    Melange,
    /// Pas assez de prix pour trancher.
    #[default]
    Inconnu,
}

/// Le sens de saisie d'une extension d'après ses témoins.
///
/// Un témoin est une carte dont deux versions ont des valeurs très écartées :
/// il dit « dans l'ordre » quand la plus précieuse a l'identifiant le plus
/// grand. 90 % des témoins dans un sens suffisent ; en deçà de
/// [`TEMOINS_MINIMUM`], on ne conclut pas.
///
/// ```
/// use ygo_core::cardmarket::{classer_sens, Sens};
/// assert_eq!(classer_sens(25, 1), Sens::DansLOrdre);
/// assert_eq!(classer_sens(0, 25), Sens::Inverse);
/// assert_eq!(classer_sens(9, 11), Sens::Melange);
/// assert_eq!(classer_sens(2, 1), Sens::Inconnu);
/// ```
#[must_use]
pub fn classer_sens(dans_l_ordre: u32, a_l_envers: u32) -> Sens {
    let total = dans_l_ordre + a_l_envers;
    if total < TEMOINS_MINIMUM {
        return Sens::Inconnu;
    }
    let part = f64::from(dans_l_ordre) / f64::from(total);
    if part >= 0.9 {
        Sens::DansLOrdre
    } else if part <= 0.1 {
        Sens::Inverse
    } else {
        Sens::Melange
    }
}

/// Les versions d'une carte : celles créées avec le set, et celles ajoutées
/// plus de [`JOURS_D_AJOUT`] jours après la première.
///
/// Les ajouts tardifs — rééditions « 2024 Version (New Card Layout) » de
/// LODT, carte géante « Oversized » de GLD4 — sont des versions que la base
/// ne connaît pas ; Cardmarket les range à côté de leur rareté, ce qui
/// décale les numéros V.n.
///
/// ```
/// use ygo_core::cardmarket::separer_ajouts;
/// let v = [(3, "2018-02-01 10:00:00"), (1, "2018-01-31 09:00:00"), (9, "2024-05-02 08:00:00")];
/// assert_eq!(separer_ajouts(&v), (vec![1, 3], vec![9]));
/// ```
#[must_use]
pub fn separer_ajouts(versions: &[(i64, &str)]) -> (Vec<i64>, Vec<i64>) {
    let date = |d: &str| {
        d.get(..10)
            .and_then(|j| chrono::NaiveDate::parse_from_str(j, "%Y-%m-%d").ok())
    };
    let premiere = versions.iter().filter_map(|(_, d)| date(d)).min();
    let mut origine = Vec::new();
    let mut ajouts = Vec::new();
    for (id, d) in versions {
        let tardif = match (premiere, date(d)) {
            (Some(p), Some(x)) => (x - p).num_days() > JOURS_D_AJOUT,
            _ => false,
        };
        if tardif {
            ajouts.push(*id);
        } else {
            origine.push(*id);
        }
    }
    origine.sort_unstable();
    ajouts.sort_unstable();
    (origine, ajouts)
}

/// Au-delà, une version est un ajout tardif plutôt qu'une version du set.
pub const JOURS_D_AJOUT: i64 = 30;

// ─────────────────────────────────────────────────────────────────────────────
// Adresses
// ─────────────────────────────────────────────────────────────────────────────

/// La langue du site selon celle de l'interface.
#[must_use]
pub fn langue_du_site(francais: bool) -> &'static str {
    if francais {
        "fr"
    } else {
        "en"
    }
}

/// La langue des annonces sur laquelle ouvrir la page d'une carte
/// (2026-10-23).
///
/// Chez Cardmarket, une carte TCG n'a qu'**une** page pour toutes les langues
/// occidentales (`RA05-FR136` et `RA05-EN136` sont le même produit) : la
/// langue est celle de chaque annonce. Les prix des fichiers publics les
/// mélangent toutes ; la page, elle, se filtre — c'est ce que fait ce
/// réglage, sans rien changer aux prix affichés dans l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LangueDesAnnonces {
    /// Pas de filtre.
    Toutes,
    /// Annonces en anglais.
    Anglais,
    /// Annonces en français — le défaut : la collection de l'utilisateur.
    #[default]
    Francais,
    /// Annonces en allemand.
    Allemand,
    /// Annonces en italien.
    Italien,
    /// Annonces en espagnol.
    Espagnol,
    /// Annonces en portugais.
    Portugais,
}

impl LangueDesAnnonces {
    /// Toutes, dans l'ordre de la liste déroulante.
    #[must_use]
    pub fn toutes() -> [Self; 7] {
        [
            Self::Francais,
            Self::Anglais,
            Self::Allemand,
            Self::Italien,
            Self::Espagnol,
            Self::Portugais,
            Self::Toutes,
        ]
    }

    /// Le code enregistré (`app_config.json`, `meta` du classeur).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Toutes => "toutes",
            Self::Anglais => "en",
            Self::Francais => "fr",
            Self::Allemand => "de",
            Self::Italien => "it",
            Self::Espagnol => "es",
            Self::Portugais => "pt",
        }
    }

    /// Relit un code enregistré — `None` s'il est inconnu.
    ///
    /// ```
    /// use ygo_core::cardmarket::LangueDesAnnonces;
    /// for l in LangueDesAnnonces::toutes() {
    ///     assert_eq!(LangueDesAnnonces::depuis_code(l.code()), Some(l));
    /// }
    /// assert_eq!(LangueDesAnnonces::depuis_code(" FR "), Some(LangueDesAnnonces::Francais));
    /// assert_eq!(LangueDesAnnonces::depuis_code("klingon"), None);
    /// ```
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        Self::toutes().into_iter().find(|l| l.code() == code)
    }

    /// Le libellé de la liste déroulante.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Toutes => "toutes langues",
            Self::Anglais => "en anglais",
            Self::Francais => "en français",
            Self::Allemand => "en allemand",
            Self::Italien => "en italien",
            Self::Espagnol => "en espagnol",
            Self::Portugais => "en portugais",
        }
    }

    /// L'identifiant de la langue chez Cardmarket, paramètre `language` de
    /// la page d'un produit — `None` sans filtre.
    #[must_use]
    pub fn id_cardmarket(self) -> Option<u8> {
        match self {
            Self::Toutes => None,
            Self::Anglais => Some(1),
            Self::Francais => Some(2),
            Self::Allemand => Some(3),
            Self::Espagnol => Some(4),
            Self::Italien => Some(5),
            Self::Portugais => Some(8),
        }
    }
}

/// Le filtre de langue à poser sur la page d'une carte : l'identifiant
/// Cardmarket de `langue`, sauf pour une carte asiatique (`CORI-JP001`,
/// `…-KR…`), qui a ses propres produits, en une seule langue.
///
/// ```
/// use ygo_core::cardmarket::{filtre_des_annonces, LangueDesAnnonces};
/// let fr = LangueDesAnnonces::Francais;
/// assert_eq!(filtre_des_annonces("RA05-EN136", fr), Some(2));
/// assert_eq!(filtre_des_annonces("RA04-FR052", fr), Some(2));
/// assert_eq!(filtre_des_annonces("LDK2-ENK22", fr), Some(2));
/// assert_eq!(filtre_des_annonces("SDK-001", fr), Some(2), "ancien code TCG");
/// assert_eq!(filtre_des_annonces("CORI-JP001", fr), None);
/// assert_eq!(filtre_des_annonces("LOCR-KR001", fr), None);
/// assert_eq!(filtre_des_annonces("RA05-EN136", LangueDesAnnonces::Toutes), None);
/// ```
#[must_use]
pub fn filtre_des_annonces(set_code: &str, langue: LangueDesAnnonces) -> Option<u8> {
    /// Les langues dont les cartes ont leurs propres produits.
    const ASIATIQUES: [&str; 7] = ["JP", "JA", "KR", "AE", "TC", "SC", "CN"];
    let code = set_code.trim().to_ascii_uppercase();
    let asiatique = code
        .rsplit_once('-')
        .and_then(|(_, reste)| reste.get(..2))
        .is_some_and(|l| ASIATIQUES.contains(&l));
    if asiatique {
        return None;
    }
    langue.id_cardmarket()
}

/// La page d'un produit, ouverte sur les annonces d'une langue
/// ([`filtre_des_annonces`]) — sans filtre, [`url_produit`].
///
/// ```
/// use ygo_core::cardmarket::url_produit_filtre;
/// assert_eq!(
///     url_produit_filtre("fr", 782049, Some(2)),
///     "https://www.cardmarket.com/fr/YuGiOh/Products?idProduct=782049&language=2"
/// );
/// assert_eq!(
///     url_produit_filtre("fr", 782049, None),
///     "https://www.cardmarket.com/fr/YuGiOh/Products?idProduct=782049"
/// );
/// ```
#[must_use]
pub fn url_produit_filtre(langue: &str, id_produit: i64, annonces: Option<u8>) -> String {
    let url = url_produit(langue, id_produit);
    match annonces {
        Some(id) => format!("{url}&language={id}"),
        None => url,
    }
}

/// La page d'un produit. Cardmarket redirige vers l'adresse canonique
/// (`…/Singles/<Extension>/<Carte>-V2-Secret-Rare`) en gardant les filtres.
///
/// ```
/// use ygo_core::cardmarket::url_produit;
/// assert_eq!(url_produit("fr", 782049), "https://www.cardmarket.com/fr/YuGiOh/Products?idProduct=782049");
/// ```
#[must_use]
pub fn url_produit(langue: &str, id_produit: i64) -> String {
    format!("{SITE}/{langue}/YuGiOh/Products?idProduct={id_produit}")
}

/// La recherche d'une carte, restreinte à une extension si on la connaît :
/// la page liste alors chaque version avec son numéro V.n et son prix.
///
/// ```
/// use ygo_core::cardmarket::url_recherche;
/// assert_eq!(
///     url_recherche("fr", "Dark Magician, the Pharaoh's Servant", Some(6508)),
///     "https://www.cardmarket.com/fr/YuGiOh/Products/Search?searchString=Dark+Magician%2C+the+Pharaoh%27s+Servant&idExpansion=6508"
/// );
/// ```
#[must_use]
pub fn url_recherche(langue: &str, nom: &str, id_extension: Option<i64>) -> String {
    let mut url = format!(
        "{SITE}/{langue}/YuGiOh/Products/Search?searchString={}",
        encoder(nom)
    );
    if let Some(id) = id_extension {
        url.push_str(&format!("&idExpansion={id}"));
    }
    url
}

/// La recherche d'un numéro (`CORI-JP033`), dans le mode de recherche que le
/// bouton « Trouver » du site emploie.
///
/// Sans `searchMode=v2`, Cardmarket ne cherche que dans les noms : la page
/// d'un numéro reste vide (relevé de l'utilisateur, 2026-10-23 :
/// `Search?searchString=CORI-JP033` ne montre rien ; « Trouver » mène à
/// `Search?searchMode=v2&idCategory=0&idExpansion=0&searchString=CORI-JP033&idRarity=0&perSite=30`,
/// qui montre la carte). Les paramètres sont repris tels que le site les
/// écrit.
///
/// ```
/// use ygo_core::cardmarket::url_recherche_numero;
/// assert_eq!(
///     url_recherche_numero("fr", "CORI-JP033"),
///     "https://www.cardmarket.com/fr/YuGiOh/Products/Search?searchMode=v2&idCategory=0&idExpansion=0&searchString=CORI-JP033&idRarity=0&perSite=30"
/// );
/// ```
#[must_use]
pub fn url_recherche_numero(langue: &str, numero: &str) -> String {
    format!(
        "{SITE}/{langue}/YuGiOh/Products/Search?searchMode=v2&idCategory=0&idExpansion=0\
         &searchString={}&idRarity=0&perSite=30",
        encoder(numero)
    )
}

/// Le numéro à chercher chez Cardmarket quand le nom ne mène nulle part :
/// celui de la ligne, ramené à l'anglais pour une carte TCG — Cardmarket
/// range toutes les langues occidentales sous un seul produit, au numéro
/// anglais (`RA04-FR052` → `RA04-EN052`) —, tel quel pour une carte OCG
/// (`CORI-JP001`). `None` si le code n'a pas la forme `PRÉFIXE-LLnnn` — `LL`
/// la langue, `nnn` le numéro, éventuellement précédé de lettres
/// (`LDK2-ENK22`, `CORI-JPS01`) : on cherchera le nom.
///
/// Un numéro TCG à plusieurs versions ouvre directement **une** d'elles, pas
/// leur liste (relevé de l'utilisateur, octobre 2026 : `RA04-EN052` ouvre la
/// V.6 de « El Shaddoll Winda », `RA04-EN106` la V.2 de « Dark Magician ») :
/// le numéro ne sert qu'en dernier recours, quand l'extension ou la carte
/// n'ont pas été retrouvées. Quand seule la version est douteuse, la recherche
/// du nom dans l'extension, qui liste toutes les versions, vaut mieux.
///
/// ```
/// use ygo_core::cardmarket::numero_de_recherche;
/// assert_eq!(numero_de_recherche("CORI-JP001").as_deref(), Some("CORI-JP001"));
/// assert_eq!(numero_de_recherche(" cori-jps01 ").as_deref(), Some("CORI-JPS01"));
/// assert_eq!(numero_de_recherche("RA04-FR052").as_deref(), Some("RA04-EN052"));
/// assert_eq!(numero_de_recherche("LDK2-DEK22").as_deref(), Some("LDK2-ENK22"));
/// assert_eq!(numero_de_recherche("SDWD-EN001").as_deref(), Some("SDWD-EN001"));
/// assert_eq!(numero_de_recherche("PROMO"), None);
/// assert_eq!(numero_de_recherche("RA04-FR"), None);
/// assert_eq!(numero_de_recherche("SDK-001"), None);
/// ```
#[must_use]
pub fn numero_de_recherche(set_code: &str) -> Option<String> {
    /// Les langues occidentales, en plus de l'anglais.
    const EUROPEENNES: [&str; 6] = ["FR", "DE", "IT", "PT", "SP", "ES"];
    let code = set_code.trim().to_ascii_uppercase();
    let (prefixe, reste) = code.rsplit_once('-')?;
    let langue = reste.get(..2)?;
    let numero = reste.get(2..)?;
    let lettres = numero.chars().take_while(char::is_ascii_uppercase).count();
    let chiffres = numero.get(lettres..)?;
    let forme = !prefixe.is_empty()
        && langue.chars().all(|c| c.is_ascii_uppercase())
        && !chiffres.is_empty()
        && chiffres.chars().all(|c| c.is_ascii_digit());
    if !forme {
        return None;
    }
    let langue = if EUROPEENNES.contains(&langue) {
        "EN"
    } else {
        langue
    };
    Some(format!("{prefixe}-{langue}{numero}"))
}

/// Encodage d'un paramètre de requête : espaces en `+`, le reste hors
/// caractères non réservés en `%XX`.
fn encoder(texte: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut sortie = String::with_capacity(texte.len() * 3);
    for octet in texte.bytes() {
        match octet {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                sortie.push(char::from(octet));
            }
            b' ' => sortie.push('+'),
            _ => {
                sortie.push('%');
                for q in [octet >> 4, octet & 0x0F] {
                    if let Some(h) = HEX.get(usize::from(q)) {
                        sortie.push(char::from(*h));
                    }
                }
            }
        }
    }
    sortie
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    fn ctx(id: i64, region: Region, sortie: Option<&str>) -> ContexteOrdre<'_> {
        ContexteOrdre {
            id_extension: id,
            region,
            sortie,
        }
    }

    /// Les Rarity Collections TCG — elles seules rangent Quarter Century et
    /// Starlight avant la Collector's, et elles sont toutes confirmées.
    const RARITY_COLLECTIONS: [i64; 5] = [5404, 5604, 5848, 5989, 6424];

    /// Les sets confirmés : la règle générale ne contredit aucun d'eux, à une
    /// exception près, nommée. C'est la propriété qui a fait retenir le
    /// modèle (octobre 2026). Elle est vérifiée sur les clés de version
    /// aussi : `avant` sur une extension sans ordre relevé (`0`), c'est le
    /// modèle, clés comprises.
    #[test]
    fn le_modele_ne_contredit_aucun_ordre_confirme() {
        let mut ecarts = Vec::new();
        for id in extensions_confirmees() {
            for sequence in ordres_confirmes(id) {
                for (i, a) in sequence.iter().enumerate() {
                    for b in sequence.iter().skip(i + 1) {
                        for region in [Region::Tcg, Region::Ocg] {
                            if avant(a, b, &ctx(0, region, None)) == Some(false) {
                                ecarts.push((id, region, a.clone(), b.clone()));
                            }
                        }
                    }
                }
            }
        }
        // La règle TCG « Collector's avant Starlight et Quarter Century »
        // (VASM, PHRE…) ne vaut pas pour les Rarity Collections : confirmées,
        // elles ne passent jamais par le modèle.
        for (id, region, a, b) in &ecarts {
            let (ra, _) = decouper_cle(a);
            let (rb, _) = decouper_cle(b);
            assert!(
                RARITY_COLLECTIONS.contains(id)
                    && *region == Region::Tcg
                    && rb == COLLECTOR
                    && (ra == QUARTER_CENTURY || ra == STARLIGHT),
                "extension {id} ({region:?}) : le modèle place {b} avant {a}"
            );
        }
        // Et `avant`, qui consulte d'abord les confirmations, les respecte.
        for id in RARITY_COLLECTIONS {
            let c = ctx(id, Region::Tcg, None);
            for sequence in ordres_confirmes(id) {
                for (i, a) in sequence.iter().enumerate() {
                    for b in sequence.iter().skip(i + 1) {
                        assert_eq!(avant(a, b, &c), Some(true), "{id} : {a} / {b}");
                    }
                }
            }
        }
    }

    #[test]
    fn la_table_se_lit_et_couvre_les_sets_confirmes() {
        let ids = extensions_confirmees();
        assert!(ids.len() >= 30, "{} extensions", ids.len());
        assert!(ids.contains(&6508), "LOCH");
        assert!(ids.contains(&6069), "BLMM");
    }

    /// Les pages du 2026-10-21, telles que l'utilisateur les a relevées.
    #[test]
    fn les_pages_du_lot_5_sont_rendues() {
        let cotd = ordonner(
            &s(&["Ghost Rare", "Ultimate Rare", "Secret Rare", "Ultra Rare"]),
            &ctx(4644, Region::Ocg, Some("2017-04-15")),
        );
        assert_eq!(
            cotd.unwrap(),
            s(&["Ultra Rare", "Secret Rare", "Ultimate Rare", "Ghost Rare"])
        );
        let bpro = ordonner(
            &s(&["Ultimate Rare", "Prismatic Secret Rare", "Secret Rare"]),
            &ctx(6365, Region::Ocg, Some("2025-10-25")),
        );
        assert_eq!(
            bpro.unwrap(),
            s(&["Secret Rare", "Prismatic Secret Rare", "Ultimate Rare"])
        );
        let gs05 = ordonner(
            &s(&["Gold Secret Rare", "Short Print", "Gold Rare"]),
            &ctx(4727, Region::Ocg, Some("2013-01-12")),
        );
        assert_eq!(
            gs05.unwrap(),
            s(&["Short Print", "Gold Rare", "Gold Secret Rare"])
        );
    }

    /// Sans confirmation, la même paire reste inconnue en 2025 — BPRO et DUAD
    /// la rangent en sens contraires.
    #[test]
    fn deux_speciales_sans_regle_restent_inconnues() {
        let c = ctx(0, Region::Ocg, Some("2025-06-01"));
        assert_eq!(avant("Ultimate Rare", "Prismatic Secret Rare", &c), None);
        assert!(ordonner(&s(&["Ultimate Rare", "Prismatic Secret Rare"]), &c).is_none());
        // 2021 : la règle d'époque tranche.
        let c = ctx(0, Region::Ocg, Some("2021-07-17"));
        assert_eq!(
            avant("Ultimate Rare", "Prismatic Secret Rare", &c),
            Some(true)
        );
    }

    #[test]
    fn regles_d_epoque_ocg_du_quarter_century() {
        let dune = ctx(0, Region::Ocg, Some("2023-04-22"));
        assert_eq!(
            avant("Ultimate Rare", "Quarter Century Secret Rare", &dune),
            Some(true)
        );
        let rota = ctx(0, Region::Ocg, Some("2024-07-27"));
        assert_eq!(
            avant("Ultimate Rare", "Quarter Century Secret Rare", &rota),
            Some(false)
        );
        let plus_tard = ctx(0, Region::Ocg, Some("2025-10-25"));
        assert_eq!(
            avant("Ultimate Rare", "Quarter Century Secret Rare", &plus_tard),
            None
        );
    }

    #[test]
    fn la_transitivite_complete_un_ordre() {
        // Ultimate/Platinum inconnu seul, mais RA01 est confirmé.
        let r = s(&[
            "Ultimate Rare",
            "Platinum Secret Rare",
            "Super Rare",
            "Collector's Rare",
        ]);
        let o = ordonner(&r, &ctx(5404, Region::Tcg, None)).unwrap();
        assert_eq!(
            o,
            s(&[
                "Super Rare",
                "Platinum Secret Rare",
                "Collector's Rare",
                "Ultimate Rare"
            ])
        );
    }

    #[test]
    fn une_rarete_inconnue_rend_l_ordre_inconnu() {
        let r = s(&["Ultra Rare", "Rareté inventée"]);
        assert!(ordonner(&r, &ctx(0, Region::Tcg, None)).is_none());
    }

    #[test]
    fn les_parallels_suivent_leur_version_normale() {
        let c = ctx(0, Region::Ocg, None);
        assert_eq!(avant("Super Rare", "Super Parallel Rare", &c), Some(true));
        assert_eq!(
            avant("Duel Terminal Ultra Parallel Rare", "Ultra Rare", &c),
            Some(false)
        );
        assert_eq!(avant("Common Parallel Rare", "Super Rare", &c), Some(true));
    }

    #[test]
    fn noms_pleine_chasse_et_ponctuation() {
        assert_eq!(normaliser_nom("Ｎｕｍｂｅｒ ３９"), "number39");
        assert_eq!(normaliser_nom("Æther"), "aether");
        assert_eq!(normaliser_nom("  Raye  "), "raye");
        assert_eq!(normaliser_nom("K9-X \"Ripper/M\""), "k9xripperm");
    }

    #[test]
    fn encodage_des_caracteres_non_ascii() {
        assert_eq!(encoder("é"), "%C3%A9");
        assert_eq!(encoder("a b&c"), "a+b%26c");
    }

    #[test]
    fn ajouts_sans_date_lisible_restent_d_origine() {
        let v = [(5, "?"), (2, "2007-01-01 00:00:00")];
        assert_eq!(separer_ajouts(&v), (vec![2, 5], vec![]));
    }

    /// Les pages du 2026-10-21 (lot 6) : les deux Limit Over Collection
    /// rangent Ultimate et Prismatic en sens contraires.
    #[test]
    fn les_pages_du_lot_6_sont_rendues() {
        let locr = ordonner(
            &s(&[
                "Prismatic Secret Rare",
                "Ultimate Rare",
                "Super Rare",
                "Secret Rare",
            ]),
            &ctx(6512, Region::Ocg, Some("2026-03-28")),
        );
        assert_eq!(
            locr.unwrap(),
            s(&[
                "Super Rare",
                "Secret Rare",
                "Ultimate Rare",
                "Prismatic Secret Rare"
            ])
        );
        let loch = ordonner(
            &s(&[
                "Prismatic Secret Rare",
                "Ultimate Rare",
                "Super Rare",
                "Secret Rare",
            ]),
            &ctx(6508, Region::Ocg, Some("2026-03-07")),
        );
        assert_eq!(
            loch.unwrap(),
            s(&[
                "Super Rare",
                "Secret Rare",
                "Prismatic Secret Rare",
                "Ultimate Rare"
            ])
        );
    }

    /// RA02 « Rescue Cat » : sept raretés en deux illustrations, l'originale
    /// d'abord — et « Droll & Lock Bird », jamais relevée, suit la même page.
    #[test]
    fn les_illustrations_de_ra02_se_suivent_rarete_par_rarete() {
        let page = ordres_confirmes(5604)
            .iter()
            .find(|s| s.len() == 14)
            .unwrap()
            .clone();
        let mut melange = page.clone();
        melange.reverse();
        melange.swap(0, 7);
        let c = ctx(5604, Region::Tcg, Some("2024-05-23"));
        assert_eq!(ordonner(&melange, &c).unwrap(), page);
        assert_eq!(page[0], "Super Rare");
        assert_eq!(page[1], "Super Rare (illustration 2)");
        assert_eq!(page[13], "Ultimate Rare (illustration 2)");
    }

    /// Les pages du 2026-10-22 (lot 7) : RA04 « El Shaddoll Winda » met
    /// l'alternative Quarter Century juste après l'originale ; RA05
    /// « Raigeki » met ses deux alternatives en tête.
    #[test]
    fn les_pages_du_lot_7_sont_rendues() {
        let winda = s(&[
            "Ultimate Rare",
            "Quarter Century Secret Rare (illustration 2)",
            "Collector's Rare",
            "Quarter Century Secret Rare",
            "Platinum Secret Rare",
            "Secret Rare",
            "Ultra Rare",
            "Super Rare",
        ]);
        assert_eq!(
            ordonner(&winda, &ctx(5989, Region::Tcg, Some("2025-04-11"))).unwrap(),
            s(&[
                "Super Rare",
                "Ultra Rare",
                "Secret Rare",
                "Platinum Secret Rare",
                "Quarter Century Secret Rare",
                "Quarter Century Secret Rare (illustration 2)",
                "Collector's Rare",
                "Ultimate Rare",
            ])
        );
        // « Vanquish Soul Razen » a les mêmes neuf versions que « Raigeki ».
        let raigeki = s(&[
            "Collector's Rare",
            "Starlight Rare",
            "Super Rare",
            "Ultra Rare",
            "Ultra Rare (illustration 2)",
            "Secret Rare",
            "Starlight Rare (illustration 2)",
            "Platinum Secret Rare",
            "Ultimate Rare",
        ]);
        assert_eq!(
            ordonner(&raigeki, &ctx(6424, Region::Tcg, Some("2026-04-10"))).unwrap(),
            s(&[
                "Ultra Rare (illustration 2)",
                "Starlight Rare (illustration 2)",
                "Super Rare",
                "Ultra Rare",
                "Secret Rare",
                "Platinum Secret Rare",
                "Starlight Rare",
                "Ultimate Rare",
                "Collector's Rare",
            ])
        );
    }

    /// RA04 : la page de « El Shaddoll Winda » vaut pour tout le set — une
    /// alternative Platinum (« Dark Magician Girl », « El Shaddoll
    /// Construct ») suit son originale. Deux alternatives d'une même rareté
    /// (« Red-Eyes », trois illustrations) restent sans ordre.
    #[test]
    fn ra04_toute_alternative_suit_son_originale() {
        let c = ctx(5989, Region::Tcg, Some("2025-04-11"));
        let construct = s(&[
            "Quarter Century Secret Rare (illustration 2)",
            "Platinum Secret Rare (illustration 2)",
            "Quarter Century Secret Rare",
            "Platinum Secret Rare",
        ]);
        assert_eq!(
            ordonner(&construct, &c).unwrap(),
            s(&[
                "Platinum Secret Rare",
                "Platinum Secret Rare (illustration 2)",
                "Quarter Century Secret Rare",
                "Quarter Century Secret Rare (illustration 2)",
            ])
        );
        let red_eyes = s(&[
            "Platinum Secret Rare",
            "Platinum Secret Rare (illustration 2)",
            "Platinum Secret Rare (illustration 3)",
        ]);
        assert!(ordonner(&red_eyes, &c).is_none());
        assert_eq!(
            avant(
                "Platinum Secret Rare",
                "Platinum Secret Rare (illustration 3)",
                &c
            ),
            Some(true)
        );
    }

    /// Sans page (RA03), ou hors des versions relevées (RA05), une
    /// alternative n'est pas placée — RA05 contredirait la règle de RA02.
    #[test]
    fn sans_page_une_alternative_reste_inconnue() {
        let ra03 = ctx(5848, Region::Tcg, Some("2024-11-07"));
        let cles = s(&[
            "Platinum Secret Rare",
            "Platinum Secret Rare (illustration 2)",
            "Quarter Century Secret Rare",
        ]);
        assert!(ordonner(&cles, &ra03).is_none());
        assert_eq!(
            avant(
                "Platinum Secret Rare (illustration 2)",
                "Quarter Century Secret Rare",
                &ra03
            ),
            None
        );
        let ra05 = ctx(6424, Region::Tcg, Some("2026-04-10"));
        assert_eq!(
            avant("Secret Rare (illustration 2)", "Super Rare", &ra05),
            None
        );
        assert!(ordonner(
            &s(&["Super Rare", "Secret Rare", "Secret Rare (illustration 2)"]),
            &ra05
        )
        .is_none());
    }

    /// Deux pages d'un même set ne se contredisent jamais.
    #[test]
    fn les_pages_d_un_set_s_accordent() {
        for id in extensions_confirmees() {
            let sequences = ordres_confirmes(id);
            for (n, s1) in sequences.iter().enumerate() {
                for s2 in sequences.iter().skip(n + 1) {
                    for (i, a) in s1.iter().enumerate() {
                        for b in s1.iter().skip(i + 1) {
                            let (Some(x), Some(y)) = (
                                s2.iter().position(|r| r == a),
                                s2.iter().position(|r| r == b),
                            ) else {
                                continue;
                            };
                            assert!(x < y, "{id} : {a} / {b}");
                        }
                    }
                }
            }
        }
    }

    /// L'Ultra Overframe des cartes de légende ne suit pas sa rareté : LOCH
    /// et LOCR la rangent après la Secret. Ailleurs, rien ne la place.
    #[test]
    fn l_overframe_des_legendes() {
        let cles = s(&[
            "Grand Master Rare",
            "Prismatic Secret Rare",
            "Ultra Rare (Overframe)",
            "Secret Rare",
            "Ultra Rare",
        ]);
        let attendu = s(&[
            "Ultra Rare",
            "Secret Rare",
            "Ultra Rare (Overframe)",
            "Prismatic Secret Rare",
            "Grand Master Rare",
        ]);
        for id in [6508, 6512] {
            let c = ctx(id, Region::Ocg, Some("2026-03-07"));
            assert_eq!(ordonner(&cles, &c).unwrap(), attendu, "{id}");
        }
        let ailleurs = ctx(0, Region::Ocg, Some("2026-03-07"));
        assert_eq!(
            avant("Ultra Rare (Overframe)", "Secret Rare", &ailleurs),
            None
        );
        assert_eq!(
            avant("Ultra Rare", "Ultra Rare (Overframe)", &ailleurs),
            None
        );
        assert!(ordonner(&cles, &ailleurs).is_none());
    }

    #[test]
    fn les_cles_font_l_aller_retour() {
        for d in [
            Distinction::Aucune,
            Distinction::Illustration(2),
            Distinction::Illustration(6),
            Distinction::Overframe,
        ] {
            let cle = cle_de_version("Quarter Century Secret Rare", d);
            assert_eq!(decouper_cle(&cle), ("Quarter Century Secret Rare", d));
        }
        // Une parenthèse qui n'est pas une précision reste dans la rareté.
        assert_eq!(
            decouper_cle("Rare (illustration 1)"),
            ("Rare (illustration 1)", Distinction::Aucune)
        );
        assert_eq!(
            decouper_cle("Rare (Foil)"),
            ("Rare (Foil)", Distinction::Aucune)
        );
        assert_eq!(Distinction::du_suffixe("AA0"), None);
        assert_eq!(Distinction::du_suffixe("AA-EA"), None);
        assert_eq!(Distinction::du_suffixe("A"), Some(Distinction::Aucune));
        assert_eq!(Distinction::du_suffixe("1"), Some(Distinction::Aucune));
    }
}
