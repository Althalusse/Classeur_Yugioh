// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Poser les lignes d'un CSV Scanflip sur les lignes des classeurs.
//!
//! Portage de `module/import_csv/import_collection.py`, avec **une
//! divergence assumée** qui change ce que l'import écrit dans la base.
//!
//! # Le rang d'artwork se calcule par rareté
//!
//! Le Python numérote les artworks d'un `set_code` en triant ses
//! `card_image_id` distincts : rang 0 au plus petit, 1 au suivant, etc. Ce
//! calcul supposait qu'un `set_code` ait une image par artwork. La passe
//! artworks Yugipedia a rendu cette hypothèse fausse : elle crée **une
//! ligne par tirage**, chacune avec son propre identifiant d'image haché.
//!
//! Sur la base réelle, `RA02-EN001` porte 14 lignes et 8 identifiants
//! distincts — sept d'entre eux ne sont pas des artworks mais des
//! **raretés déguisées**. Le rang cesse alors de désigner un artwork, et
//! l'appariement `(set_code, rang, rareté)` ne trouve plus rien pour douze
//! des quatorze lignes. Le Python s'en sort par un repli : *si une seule
//! ligne existe pour ce `(set_code, rang)`, la prendre quelle que soit sa
//! rareté*. Mesuré sur l'export réel de l'utilisateur, ce repli attribue
//! **24 lignes à la mauvaise rareté** — une Platinum Secret se posant sur
//! une Collector's Rare, une Quarter Century sur une Super Rare.
//!
//! Ici, le rang se calcule au sein d'un même **`(set_code, rareté)`**. Chaque
//! tirage retrouve ses propres artworks, et l'appariement se fait sur la clé
//! exacte : 402 des 407 lignes réelles s'apparient **sans jamais deviner une
//! rareté**, contre 378 exactes plus 24 devinées.
//!
//! # Le rang désigne une ligne, pas une image
//!
//! Numéroter les **images distinctes** laissait un trou. `LDK2-ENK01` porte
//! **trois** lignes de classeur — Kaiba a trois Dragons Blancs aux Yeux
//! Bleus dans son deck — mais les sources leur donnent **un seul**
//! `card_image_id` : trois lignes, une image, donc un seul rang. Les
//! exemplaires 2 et 3 n'avaient aucun rang à eux, l'import les refusait
//! (`ArtworkAbsent`), et l'export les écrivait tous les trois en rang 0 —
//! un aller-retour perdait deux cartes sur trois.
//!
//! Le rang numérote donc les **lignes** du groupe `(set_code, rareté)`,
//! ordonnées par `(card_image_id, rowid)`. Trois lignes identiques
//! reçoivent les rangs 0, 1, 2 ; deux lignes d'images différentes gardent
//! les rangs que l'ancien calcul leur donnait, puisque le tri commence par
//! l'image. Chaque ligne du classeur a désormais **son** rang, et la clé
//! `(set_code, rang, rareté)` désigne une ligne et une seule.
//!
//! # Ce que la clé ne contient pas : le nom
//!
//! Les conventions typographiques divergent entre Konami FR, YGOJSON et
//! YGOPRODeck — « Ange 01 » contre « Ange O1 », « Voeux » contre « Vœux ».
//! Le triplet `(set_code, rang, rareté)` identifie déjà une carte physique
//! sans ambiguïté ; le nom du CSV ne sert donc pas à apparier — **sauf**
//! quand le code, lui, n'identifie rien.
//!
//! # Le cas des sets à sous-jeux : le nom départage
//!
//! Certains sets sont découpés en sous-jeux que Konami numérote **à part**,
//! avec une lettre entre la langue et le numéro : Legendary Decks II porte
//! `LDK2-FRJ01…J43` (deck de Joey), `LDK2-FRK01…K41` (Kaiba),
//! `LDK2-FRS01…S06` (les Dieux) et `LDK2-FRY01…Y40` (Yugi).
//!
//! Des CSV existent où cette lettre a été perdue en amont : les 132 lignes
//! LDK2 y sont écrites `LDK2-FR01`, `LDK2-FR02`… et **quatre cartes
//! différentes portent alors le même code**. Traduits en anglais, ces codes
//! ne désignent rien du tout, et les 132 lignes sont refusées en bloc alors
//! que la carte existe bel et bien dans le classeur.
//!
//! Le nom résout ce cas et lui seul. Quand un code **sans lettre de
//! sous-jeu** ne trouve rien, on cherche dans le classeur les tirages de
//! même préfixe et de même numéro, et on garde ceux dont le nom — anglais
//! ou français, comparé sans casse ni accents ni ponctuation — est celui du
//! CSV. **Un seul candidat est accepté** : deux, et la ligne est refusée
//! comme avant. Sur le fichier réel, les 132 lignes se résolvent sans une
//! seule ambiguïté.
//!
//! Ces résolutions sont **nommées dans le rapport** ([`Rapport::resolues`]) :
//! l'utilisateur voit quel code son fichier portait et lequel a été retenu.
//! Deviner en silence serait pire que refuser.
//!
//! # Ce que l'import ne fait pas
//!
//! Il n'écrit que sur les lignes qu'il a appariées. Une carte possédée dans
//! l'application mais absente du CSV **garde sa quantité** : un CSV partiel
//! n'est pas une déclaration de dépossession.

use std::collections::{BTreeMap, HashMap, HashSet};

use ygo_core::paths::Paths;
use ygo_core::rarity::reference;

use crate::error::{AppError, Result};
use crate::exemplaires::Etat;
use crate::scanflip::{Edition, Ligne};

/// Une ligne de classeur, réduite à ce que l'appariement regarde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneBase {
    /// `rowid` SQLite — l'identité d'une ligne, comme partout ailleurs.
    pub rowid: i64,
    /// Code du tirage, tel que stocké (en anglais dans la quasi-totalité).
    pub set_code: String,
    /// Libellé de rareté, canonisé par le lot « raretés ».
    pub rarity: String,
    /// Identifiant d'image — c'est lui qui distingue les artworks.
    pub card_image_id: Option<i64>,
    /// Nom anglais de la carte. N'entre dans l'appariement que par le
    /// repli des sous-jeux (cf. tête de module).
    pub name: String,
    /// Nom français, quand la base en a un. Même usage.
    pub name_fr: Option<String>,
}

/// Pourquoi une ligne du CSV n'a pas trouvé sa place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refus {
    /// Le classeur n'existe pas dans cette installation.
    ClasseurAbsent,
    /// Le code de rareté n'est pas dans le référentiel Scanflip.
    RareteInconnue,
    /// Le `set_code` est absent du classeur — carte ajoutée au set après
    /// la création du classeur, ou jeton que les sources ne listent pas.
    SetCodeAbsent,
    /// Le `set_code` est connu, mais pas ce rang d'artwork : l'artwork
    /// alternatif existe chez l'utilisateur sans être référencé.
    ArtworkAbsent,
    /// Le `set_code` et le rang sont connus, mais pas dans cette rareté.
    RareteAbsente,
}

impl Refus {
    /// Ce que le refus veut dire, en une phrase.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::ClasseurAbsent => "classeur absent de l'installation",
            Self::RareteInconnue => "code de rareté inconnu du référentiel",
            Self::SetCodeAbsent => "code carte absent du classeur",
            Self::ArtworkAbsent => "artwork alternatif non référencé pour cette carte",
            Self::RareteAbsente => "cette rareté n'existe pas pour cette carte",
        }
    }
}

/// Une ligne du CSV refusée, et pourquoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusee {
    /// Numéro de ligne dans le fichier.
    pub numero: usize,
    /// Le classeur visé.
    pub classeur: String,
    /// Le code carte du CSV.
    pub code: String,
    /// Le nom du CSV — il ne sert qu'ici.
    pub nom: String,
    /// Le code de rareté du CSV.
    pub rarete: String,
    /// Le rang d'artwork demandé.
    pub artwork: u32,
    /// La cause.
    pub refus: Refus,
}

/// Ce qu'une fusion de plusieurs lignes CSV sur une même ligne de base
/// fait perdre.
///
/// # Plus que la langue — 2026-10-01
///
/// L'état et l'édition s'y trouvaient aussi : la ligne de classeur n'en
/// gardait qu'un, celui de la première ligne du fichier. Depuis les
/// exemplaires ([`crate::exemplaires`]), chaque ligne CSV d'un autre état
/// devient un groupe d'exemplaires réglés à part ([`Ecriture::a_part`]) : il
/// n'y a plus rien à perdre de ce côté. Reste la langue, que la base ne
/// porte pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perte {
    /// Les lignes fusionnées n'ont pas toutes la même langue.
    pub langue: bool,
}

impl Perte {
    /// Y a-t-il quelque chose à signaler ?
    #[must_use]
    pub fn reelle(self) -> bool {
        self.langue
    }
}

/// Plusieurs lignes du CSV se sont posées sur la même ligne de base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fusion {
    /// Le classeur.
    pub classeur: String,
    /// La ligne de base qui les reçoit.
    pub rowid: i64,
    /// Le code carte du CSV.
    pub code: String,
    /// Les numéros de ligne fusionnés, dans l'ordre du fichier.
    pub numeros: Vec<usize>,
    /// Ce que la fusion efface — vide si les lignes étaient identiques.
    pub perte: Perte,
}

/// Une écriture à faire : une ligne de base, et ce qu'elle devient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ecriture {
    /// Le classeur.
    pub classeur: String,
    /// La ligne visée.
    pub rowid: i64,
    /// La quantité, somme des lignes CSV posées dessus.
    pub quantite: i64,
    /// L'état commun — celui de la **première** ligne du fichier.
    pub qualite: String,
    /// L'édition commune, même règle.
    pub edition: Option<Edition>,
    /// Les exemplaires d'un **autre** état que la première ligne, regroupés
    /// par état dans l'ordre du fichier : `(état, nombre)`. Ils deviennent des
    /// exemplaires réglés à part.
    pub a_part: Vec<(Etat, i64)>,
}

impl Ecriture {
    /// L'état commun, sous la forme qu'emploient les exemplaires.
    #[must_use]
    pub fn commun(&self) -> Etat {
        Etat::nouveau(&self.qualite, self.edition.map_or("", Edition::code))
    }
}

/// Une ligne dont le code manquait sa lettre de sous-jeu, et que le nom a
/// permis de rattacher à un tirage précis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Resolue {
    /// Numéro de ligne dans le fichier.
    pub numero: usize,
    /// Le classeur.
    pub classeur: String,
    /// Le code tel que le CSV l'écrit — `LDK2-FR01`.
    pub code_csv: String,
    /// Le code retenu, en anglais — `LDK2-ENJ01`.
    pub code_retenu: String,
    /// Le nom qui a servi à départager.
    pub nom: String,
}

/// Tout ce que l'import ferait, avant de l'avoir fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rapport {
    /// Les écritures, ordonnées par classeur puis par `rowid`.
    pub ecritures: Vec<Ecriture>,
    /// Les lignes qui n'ont trouvé personne.
    pub refusees: Vec<Refusee>,
    /// Les fusions, y compris celles qui ne perdent rien.
    pub fusions: Vec<Fusion>,
    /// Les codes sans lettre de sous-jeu rattachés grâce au nom.
    pub resolues: Vec<Resolue>,
    /// Nombre de lignes CSV lues.
    pub lues: usize,
}

impl Rapport {
    /// Combien de lignes du CSV se sont posées quelque part.
    #[must_use]
    pub fn appariees(&self) -> usize {
        self.lues - self.refusees.len()
    }

    /// Les fusions qui effacent quelque chose.
    pub fn fusions_avec_perte(&self) -> impl Iterator<Item = &Fusion> {
        self.fusions.iter().filter(|f| f.perte.reelle())
    }

    /// Les classeurs que le CSV nomme et que l'installation n'a pas.
    ///
    /// Triés et dédoublonnés : c'est la liste à proposer à la création.
    #[must_use]
    pub fn classeurs_absents(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .refusees
            .iter()
            .filter(|r| r.refus == Refus::ClasseurAbsent)
            .map(|r| r.classeur.clone())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Convertit un code carte vers sa forme anglaise.
///
/// La base stocke les tirages en anglais (`COALESCE(sp_en, sp_fr)` du côté
/// Python). Un CSV français désigne donc `SDLI-FR001` là où la base a
/// `SDLI-EN001`.
///
/// La lettre de sous-jeu entre la langue et le numéro est **conservée** :
/// `LDK2-FRJ01` (le deck de Joey) devient `LDK2-ENJ01`, jamais `LDK2-EN01`.
/// Konami ne traduit pas cette lettre.
///
/// ```
/// use ygo_app::import::vers_anglais;
/// assert_eq!(vers_anglais("SDLI-FR001"), "SDLI-EN001");
/// assert_eq!(vers_anglais("LDK2-FRJ01"), "LDK2-ENJ01");
/// assert_eq!(vers_anglais("RA02-EN001"), "RA02-EN001");
/// assert_eq!(vers_anglais("LOCH-JP001"), "LOCH-EN001");
/// ```
#[must_use]
pub fn vers_anglais(code: &str) -> String {
    /// Les codes de langue que Konami emploie dans un `set_code`.
    const LANGUES: [&str; 19] = [
        "FR", "DE", "IT", "ES", "PT", "JP", "JA", "KR", "KO", "SP", "AE", "SC", "NA", "EU", "AU",
        "AS", "TC", "TF", "TG",
    ];
    let Some(tiret) = code.rfind('-') else {
        return code.to_owned();
    };
    let (prefixe, reste) = code.split_at(tiret + 1);
    let Some(langue) = LANGUES
        .iter()
        .find(|l| reste.len() >= 2 && reste[..2].eq_ignore_ascii_case(l))
    else {
        return code.to_owned();
    };
    let suffixe = reste.get(langue.len()..).unwrap_or("");
    // Le suffixe doit être des lettres puis au moins un chiffre : c'est ce
    // qui distingue « LDK2-FRJ01 » d'un code dont les deux premières
    // lettres ressemblent à une langue par hasard.
    let lettres = suffixe.chars().take_while(char::is_ascii_uppercase).count();
    let chiffres = suffixe.get(lettres..).unwrap_or("");
    if chiffres.is_empty() || !chiffres.chars().all(|c| c.is_ascii_digit()) {
        return code.to_owned();
    }
    format!("{prefixe}EN{suffixe}")
}

/// Le préfixe et le numéro d'un code **dépourvu de lettre de sous-jeu**.
///
/// Rend `None` dès qu'il y a une lettre entre la langue et le numéro : ces
/// codes-là sont complets et n'ont rien à réparer.
///
/// ```
/// use ygo_app::import::sans_sous_jeu;
/// assert_eq!(sans_sous_jeu("LDK2-FR01"), Some(("LDK2-", "01")));
/// assert_eq!(sans_sous_jeu("LDK2-ENJ01"), None);
/// assert_eq!(sans_sous_jeu("RA02-FR001"), Some(("RA02-", "001")));
/// assert_eq!(sans_sous_jeu("PROMO"), None);
/// ```
#[must_use]
pub fn sans_sous_jeu(code: &str) -> Option<(&str, &str)> {
    let tiret = code.rfind('-')?;
    let (prefixe, reste) = code.split_at(tiret + 1);
    if reste.len() < 3 {
        return None;
    }
    let (langue, chiffres) = reste.split_at(2);
    if !langue.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if chiffres.is_empty() || !chiffres.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((prefixe, chiffres))
}

/// Le numéro d'un code, lettre de sous-jeu comprise ou non.
///
/// `LDK2-ENJ01` et `LDK2-FR01` rendent tous deux `"01"` : c'est ce qui
/// permet de les rapprocher.
fn numero_de(code: &str) -> Option<&str> {
    let tiret = code.rfind('-')?;
    let reste = code.get(tiret + 1..)?;
    let debut = reste.find(|c: char| c.is_ascii_digit())?;
    let chiffres = reste.get(debut..)?;
    if chiffres.chars().all(|c| c.is_ascii_digit()) {
        Some(chiffres)
    } else {
        None
    }
}

/// Un nom de carte réduit à ce qui ne varie pas d'une source à l'autre.
///
/// Minuscules, accents ôtés, tout ce qui n'est ni lettre ni chiffre
/// supprimé. « Woùf es-tu ? » et « Wouf es-tu ? » se rejoignent ainsi, de
/// même que « Slifer, le Dragon Céleste » et « Slifer le Dragon Celeste ».
///
/// La décomposition est faite à la main sur les caractères latins que
/// Konami emploie — le crate `unicode-normalization` n'est pas dans les
/// dépendances, et la liste tient en une ligne.
#[must_use]
pub fn normaliser_nom(nom: &str) -> String {
    /// Les lettres accentuées rencontrées dans les noms français, et leur
    /// équivalent sans signe.
    const ACCENTS: [(char, char); 34] = [
        ('à', 'a'),
        ('á', 'a'),
        ('â', 'a'),
        ('ä', 'a'),
        ('ã', 'a'),
        ('å', 'a'),
        ('ç', 'c'),
        ('è', 'e'),
        ('é', 'e'),
        ('ê', 'e'),
        ('ë', 'e'),
        ('ì', 'i'),
        ('í', 'i'),
        ('î', 'i'),
        ('ï', 'i'),
        ('ñ', 'n'),
        ('ò', 'o'),
        ('ó', 'o'),
        ('ô', 'o'),
        ('ö', 'o'),
        ('õ', 'o'),
        ('ù', 'u'),
        ('ú', 'u'),
        ('û', 'u'),
        ('ü', 'u'),
        ('ý', 'y'),
        ('ÿ', 'y'),
        ('À', 'a'),
        ('Â', 'a'),
        ('Ç', 'c'),
        ('É', 'e'),
        ('È', 'e'),
        ('Ê', 'e'),
        ('Î', 'i'),
    ];
    let mut sortie = String::with_capacity(nom.len());
    for c in nom.chars() {
        let c = ACCENTS
            .iter()
            .find(|(accentue, _)| *accentue == c)
            .map_or(c, |(_, nu)| *nu);
        // « Œ » et « æ » se déplient plutôt qu'ils ne se dénudent.
        match c {
            'œ' | 'Œ' => sortie.push_str("oe"),
            'æ' | 'Æ' => sortie.push_str("ae"),
            _ if c.is_ascii_alphanumeric() => sortie.push(c.to_ascii_lowercase()),
            _ if c.is_alphanumeric() => sortie.extend(c.to_lowercase()),
            _ => {}
        }
    }
    sortie
}

/// L'index de repli d'un classeur : `(numéro, nom normalisé)` → `set_code`.
///
/// Ne retient que les tirages **porteurs d'une lettre de sous-jeu** : ce
/// sont les seuls qu'un code amputé puisse désigner. Un numéro dont deux
/// tirages portent le même nom n'entre pas dans l'index — l'ambiguïté est
/// exclue à la construction plutôt que constatée à l'usage.
#[must_use]
pub fn index_par_nom(base: &[LigneBase]) -> HashMap<(String, String), String> {
    let mut vus: HashMap<(String, String), HashSet<&str>> = HashMap::new();
    for l in base {
        if l.set_code.is_empty() || sans_sous_jeu(&l.set_code).is_some() {
            continue;
        }
        let Some(numero) = numero_de(&l.set_code) else {
            continue;
        };
        for nom in std::iter::once(&l.name).chain(l.name_fr.iter()) {
            let norme = normaliser_nom(nom);
            if norme.is_empty() {
                continue;
            }
            vus.entry((numero.to_owned(), norme))
                .or_default()
                .insert(l.set_code.as_str());
        }
    }
    vus.into_iter()
        .filter_map(|(cle, codes)| {
            let mut it = codes.into_iter();
            let seul = it.next()?;
            it.next().is_none().then(|| (cle, seul.to_owned()))
        })
        .collect()
}

/// L'index d'appariement d'un classeur : `(set_code, rang, rareté)` → ligne.
///
/// Le rang est calculé **par `(set_code, rareté)`** — c'est la divergence
/// décrite en tête de module — et il numérote les **lignes**, ordonnées par
/// `(card_image_id, rowid)`. La clé désigne donc une ligne et une seule :
/// deux lignes ne peuvent pas se disputer un rang, et aucune ligne ne reste
/// sans rang.
#[must_use]
pub fn index(base: &[LigneBase]) -> HashMap<(String, u32, String), i64> {
    // Les lignes de chaque `(set_code, rareté)`, dans une carte ordonnée :
    // le rang doit être le même à chaque exécution.
    let mut groupes: BTreeMap<(&str, &str), Vec<(i64, i64)>> = BTreeMap::new();
    for l in base {
        if l.set_code.is_empty() {
            continue;
        }
        groupes
            .entry((l.set_code.as_str(), l.rarity.as_str()))
            .or_default()
            .push((l.card_image_id.unwrap_or(0), l.rowid));
    }
    let mut index = HashMap::new();
    for ((set_code, rarity), mut lignes) in groupes {
        // L'image d'abord : deux artworks gardent ainsi l'ordre que leur
        // donnait l'ancien calcul. Le `rowid` ensuite, pour départager les
        // lignes qu'une même image ne distingue pas.
        lignes.sort_unstable();
        for (rang, (_, rowid)) in lignes.into_iter().enumerate() {
            let rang = u32::try_from(rang).unwrap_or(u32::MAX);
            index.insert((set_code.to_owned(), rang, rarity.to_owned()), rowid);
        }
    }
    index
}

/// Les libellés qu'une case « Rareté » peut désigner en base.
///
/// # Ce que la case peut contenir
///
/// Scanflip n'exige pas le code : sa page d'aide accepte le libellé complet
/// (« Secrète Rare »), et le libellé amputé de son « Rare » final
/// (« Ultra », « Super »). Un CSV retouché à la main, ou produit par un
/// autre outil, emploie ces formes-là. La case est donc d'abord ramenée à
/// un code par [`ygo_core::rarity::scanflip::code_de`], puis le code est
/// traduit en libellés de base.
///
/// # Pourquoi plusieurs libellés pour un code
///
/// Le libellé canonique d'abord, puis les alias du référentiel. `C` couvre
/// ainsi `Common`, mais aussi `Short Print` et `Super Short Print` — que la
/// canonisation garde distincts à dessein, et que Scanflip confond sous un
/// seul code. Essayer les alias **dans l'ordre déclaré** évite d'avoir à
/// deviner : la liste des candidats est écrite dans le référentiel, pas
/// inférée d'une ressemblance.
#[must_use]
pub fn libelles_possibles(ecriture: &str) -> Vec<String> {
    // `STRB` est dans les règles de Scanflip mais pas dans la table du
    // Python : le code est reconnu, et c'est le libellé qui manque. On se
    // rabat alors sur le libellé français de Scanflip, qui a toutes ses
    // chances de figurer en base si la carte y est.
    let code = ygo_core::rarity::scanflip::code_de(ecriture).unwrap_or(ecriture.trim());
    let mut noms: Vec<String> = reference::table()
        .iter()
        .find(|r| r.code == code)
        .map(|r| {
            let mut noms = vec![r.en.clone()];
            noms.extend(r.alias.iter().cloned());
            noms.push(r.fr.clone());
            noms
        })
        .unwrap_or_default();
    if let Some(r) = ygo_core::rarity::scanflip::table()
        .iter()
        .find(|r| r.code == code)
    {
        if !noms.contains(&r.fr) {
            noms.push(r.fr.clone());
        }
    }
    noms
}

/// Ce que l'import ferait — sans toucher à quoi que ce soit.
///
/// `bases` associe à chaque code de classeur ses lignes. Un classeur absent
/// de la carte fait refuser ses lignes en bloc.
#[must_use]
pub fn planifier(lignes: &[Ligne], bases: &HashMap<String, Vec<LigneBase>>) -> Rapport {
    let mut rapport = Rapport {
        lues: lignes.len(),
        ..Rapport::default()
    };
    // Un index par classeur, construit une fois.
    let index_par_classeur: HashMap<&String, _> = bases
        .iter()
        .map(|(code, base)| (code, index(base)))
        .collect();
    // Et son index de repli, pour les codes privés de leur lettre.
    let noms_par_classeur: HashMap<&String, _> = bases
        .iter()
        .map(|(code, base)| (code, index_par_nom(base)))
        .collect();

    // Les lignes retenues, groupées par ligne de base visée. `BTreeMap`
    // pour que l'ordre du rapport ne dépende pas d'un hachage.
    let mut groupes: BTreeMap<(String, i64), Vec<&Ligne>> = BTreeMap::new();

    for ligne in lignes {
        let refuser = |refus| Refusee {
            numero: ligne.numero,
            classeur: ligne.extension.clone(),
            code: ligne.code.clone(),
            nom: ligne.nom.clone(),
            rarete: ligne.rarete.clone(),
            artwork: ligne.artwork,
            refus,
        };
        let Some(index) = index_par_classeur.get(&ligne.extension) else {
            rapport.refusees.push(refuser(Refus::ClasseurAbsent));
            continue;
        };
        let libelles = libelles_possibles(&ligne.rarete);
        if libelles.is_empty() {
            rapport.refusees.push(refuser(Refus::RareteInconnue));
            continue;
        }
        // Le code anglais d'abord ; le code brut ensuite, pour les
        // classeurs anciens qui ont stocké un tirage non anglais.
        let anglais = vers_anglais(&ligne.code);
        let codes: Vec<&str> = if anglais == ligne.code {
            vec![ligne.code.as_str()]
        } else {
            vec![anglais.as_str(), ligne.code.as_str()]
        };

        let poser = |index: &HashMap<(String, u32, String), i64>, codes: &[&str]| {
            codes.iter().find_map(|code| {
                libelles.iter().find_map(|libelle| {
                    index
                        .get(&((*code).to_owned(), ligne.artwork, libelle.clone()))
                        .copied()
                })
            })
        };

        if let Some(rowid) = poser(index, &codes) {
            groupes
                .entry((ligne.extension.clone(), rowid))
                .or_default()
                .push(ligne);
            continue;
        }

        // Repli des sous-jeux : le code n'a pas de lettre, le nom en
        // désigne une. Cf. tête de module.
        let repli = sans_sous_jeu(&ligne.code).and_then(|(_, numero)| {
            let noms = noms_par_classeur.get(&ligne.extension)?;
            noms.get(&(numero.to_owned(), normaliser_nom(&ligne.nom)))
                .cloned()
        });
        if let Some(code_retenu) = repli {
            if let Some(rowid) = poser(index, &[code_retenu.as_str()]) {
                rapport.resolues.push(Resolue {
                    numero: ligne.numero,
                    classeur: ligne.extension.clone(),
                    code_csv: ligne.code.clone(),
                    code_retenu,
                    nom: ligne.nom.clone(),
                });
                groupes
                    .entry((ligne.extension.clone(), rowid))
                    .or_default()
                    .push(ligne);
                continue;
            }
            // Le tirage existe, mais pas dans ce rang ou cette rareté : le
            // refus doit parler du code retenu, pas du code amputé.
            let sets_rangs: HashSet<(&str, u32)> =
                index.keys().map(|(s, r, _)| (s.as_str(), *r)).collect();
            let refus = if sets_rangs.contains(&(code_retenu.as_str(), ligne.artwork)) {
                Refus::RareteAbsente
            } else {
                Refus::ArtworkAbsent
            };
            rapport.refusees.push(refuser(refus));
            continue;
        }

        // Rien trouvé : dire laquelle des trois dimensions manque.
        let sets: HashSet<&str> = index.keys().map(|(s, _, _)| s.as_str()).collect();
        let sets_rangs: HashSet<(&str, u32)> =
            index.keys().map(|(s, r, _)| (s.as_str(), *r)).collect();
        let refus = if !codes.iter().any(|c| sets.contains(c)) {
            Refus::SetCodeAbsent
        } else if !codes
            .iter()
            .any(|c| sets_rangs.contains(&(c, ligne.artwork)))
        {
            Refus::ArtworkAbsent
        } else {
            Refus::RareteAbsente
        };
        rapport.refusees.push(refuser(refus));
    }

    for ((classeur, rowid), lignes) in groupes {
        let Some(premiere) = lignes.first() else {
            continue;
        };
        if lignes.len() > 1 {
            let perte = Perte {
                langue: lignes.iter().any(|l| l.langue != premiere.langue),
            };
            rapport.fusions.push(Fusion {
                classeur: classeur.clone(),
                rowid,
                code: premiere.code.clone(),
                numeros: lignes.iter().map(|l| l.numero).collect(),
                perte,
            });
        }
        let etat_de = |l: &Ligne| Etat::nouveau(&l.qualite, l.edition.map_or("", Edition::code));
        let commun = etat_de(premiere);
        let mut a_part: Vec<(Etat, i64)> = Vec::new();
        for l in lignes.iter().skip(1) {
            let etat = etat_de(l);
            if etat == commun {
                continue;
            }
            match a_part.iter_mut().find(|(e, _)| *e == etat) {
                Some((_, n)) => *n += l.quantite,
                None => a_part.push((etat, l.quantite)),
            }
        }
        rapport.ecritures.push(Ecriture {
            classeur,
            rowid,
            quantite: lignes.iter().map(|l| l.quantite).sum(),
            // La première ligne du fichier gagne. Le Python laissait la
            // dernière écriture l'emporter, dans un ordre de dictionnaire —
            // deux exécutions pouvaient différer. Ici c'est l'ordre du
            // fichier, que l'utilisateur peut lire.
            qualite: premiere.qualite.clone(),
            edition: premiere.edition,
            a_part,
        });
    }
    rapport
}

/// Lit les lignes d'un classeur, pour l'appariement.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn lignes_de_base(paths: &Paths, classeur: &str) -> Result<Vec<LigneBase>> {
    let chemin = paths.classeur_db(classeur);
    let conn = ygo_db::connexion::ouvrir_lecture_seule(chemin)
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let mut requete = conn
        .prepare("SELECT rowid, set_code, rarity, card_image_id, name, name_fr FROM cards")
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let lignes = requete
        .query_map([], |l| {
            Ok(LigneBase {
                rowid: l.get(0)?,
                set_code: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                rarity: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                card_image_id: l.get(3)?,
                name: l.get::<_, Option<String>>(4)?.unwrap_or_default(),
                name_fr: l.get(5)?,
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    Ok(lignes)
}

/// Lit toutes les bases dont le CSV a besoin.
///
/// Un classeur nommé par le CSV mais absent du disque n'est pas une erreur :
/// il manque simplement de la carte, et ses lignes seront refusées avec
/// [`Refus::ClasseurAbsent`].
#[must_use]
pub fn bases_pour(paths: &Paths, lignes: &[Ligne]) -> HashMap<String, Vec<LigneBase>> {
    let mut vus: Vec<&str> = lignes.iter().map(|l| l.extension.as_str()).collect();
    vus.sort_unstable();
    vus.dedup();
    vus.into_iter()
        .filter(|c| paths.classeur_db(c).is_file())
        .filter_map(|c| lignes_de_base(paths, c).ok().map(|b| (c.to_owned(), b)))
        .collect()
}

/// Les classeurs qu'un CSV nomme et que l'installation n'a pas encore.
///
/// Se lit avant tout appariement : un classeur absent fait refuser toutes
/// ses lignes, et le créer d'abord change complètement le résultat.
#[must_use]
pub fn classeurs_manquants(paths: &Paths, lignes: &[Ligne]) -> Vec<String> {
    let mut v: Vec<String> = lignes
        .iter()
        .map(|l| l.extension.trim().to_uppercase())
        .filter(|c| !c.is_empty() && !paths.classeur_db(c).is_file())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Ce qu'une application a réellement écrit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Lignes de base mises à jour.
    pub ecrites: usize,
    /// Classeurs touchés.
    pub classeurs: Vec<String>,
}

/// Applique un rapport : une transaction par classeur.
///
/// Une transaction **par classeur** et non une par ligne : un import de
/// quatre cents lignes ferait quatre cents `fsync`, et un import
/// interrompu laisserait un classeur à moitié écrit sans qu'on sache
/// lequel.
///
/// # Errors
///
/// Rend une erreur si un classeur ne peut pas être ouvert ou écrit. Les
/// classeurs déjà traités gardent leurs écritures — chacun est atomique
/// pour lui-même.
pub fn appliquer(paths: &Paths, rapport: &Rapport) -> Result<Bilan> {
    let mut par_classeur: BTreeMap<&str, Vec<&Ecriture>> = BTreeMap::new();
    for e in &rapport.ecritures {
        par_classeur.entry(e.classeur.as_str()).or_default().push(e);
    }
    let mut bilan = Bilan::default();
    for (classeur, ecritures) in par_classeur {
        let mut conn = ygo_db::connexion::ouvrir(paths.classeur_db(classeur))
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        let transaction = conn
            .transaction()
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        for e in &ecritures {
            // L'import **remplace** : la quantité, l'état commun, et les
            // exemplaires à part que le fichier décrit — ceux d'avant partent.
            crate::exemplaires::remplacer(
                &transaction,
                e.rowid,
                e.quantite,
                &e.commun(),
                &e.a_part,
            )
            .map_err(|err| AppError::Creation(format!("{classeur} rowid {}: {err}", e.rowid)))?;
        }
        transaction
            .commit()
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        bilan.ecrites += ecritures.len();
        bilan.classeurs.push(classeur.to_owned());
    }
    Ok(bilan)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::scanflip;

    fn ligne(numero: usize, code: &str, rarete: &str, artwork: u32) -> Ligne {
        Ligne {
            numero,
            langue: "Français (France)".into(),
            extension: code.split('-').next().unwrap_or("").to_owned(),
            code: code.to_owned(),
            nom: "peu importe".into(),
            rarete: rarete.to_owned(),
            edition: Some(Edition::Premiere),
            qualite: "NM".into(),
            quantite: 1,
            artwork,
            reprint: String::new(),
        }
    }

    fn base(rows: &[(i64, &str, &str, i64)]) -> Vec<LigneBase> {
        rows.iter()
            .map(|(rowid, sc, ra, img)| LigneBase {
                rowid: *rowid,
                set_code: (*sc).to_owned(),
                rarity: (*ra).to_owned(),
                card_image_id: Some(*img),
                name: String::new(),
                name_fr: None,
            })
            .collect()
    }

    /// Comme [`base`], mais avec les noms — le repli des sous-jeux ne
    /// regarde que ceux-là.
    fn base_nommee(rows: &[(i64, &str, &str, i64, &str, &str)]) -> Vec<LigneBase> {
        rows.iter()
            .map(|(rowid, sc, ra, img, n, nf)| LigneBase {
                rowid: *rowid,
                set_code: (*sc).to_owned(),
                rarity: (*ra).to_owned(),
                card_image_id: Some(*img),
                name: (*n).to_owned(),
                name_fr: (!nf.is_empty()).then(|| (*nf).to_owned()),
            })
            .collect()
    }

    /// Le cas qui motive toute la divergence.
    ///
    /// Sept raretés d'un même tirage, chacune dotée par la passe artworks
    /// d'une image propre (identifiants négatifs), plus l'artwork
    /// YGOPRODeck partagé par les sept (identifiant positif). Le calcul du
    /// Python donnerait huit rangs dont sept ne sont que des raretés ; ici,
    /// chaque rareté a ses deux rangs à elle.
    #[test]
    fn le_rang_est_propre_a_chaque_rarete() {
        let base = base(&[
            (1, "RA02-EN001", "Collector's Rare", -1_276_398_914),
            (2, "RA02-EN001", "Super Rare", -1_074_569_406),
            (3, "RA02-EN001", "Ultra Rare", -939_225_438),
            (4, "RA02-EN001", "Collector's Rare", 14_878_871),
            (5, "RA02-EN001", "Super Rare", 14_878_871),
            (6, "RA02-EN001", "Ultra Rare", 14_878_871),
        ]);
        let idx = index(&base);
        // Chaque rareté a bien deux rangs, 0 et 1.
        for (rarete, r0, r1) in [
            ("Collector's Rare", 1, 4),
            ("Super Rare", 2, 5),
            ("Ultra Rare", 3, 6),
        ] {
            assert_eq!(
                idx.get(&("RA02-EN001".into(), 0, rarete.into())),
                Some(&r0),
                "{rarete} rang 0"
            );
            assert_eq!(
                idx.get(&("RA02-EN001".into(), 1, rarete.into())),
                Some(&r1),
                "{rarete} rang 1"
            );
        }
        // Et il n'existe aucun rang 2 : le calcul du Python en aurait cinq
        // de plus, tous faux.
        assert!(
            !idx.keys().any(|(_, r, _)| *r > 1),
            "aucun rang au-delà de 1"
        );
    }

    /// Chaque ligne du CSV se pose sur la ligne de SA rareté, jamais sur
    /// celle d'une voisine.
    #[test]
    fn chaque_rarete_va_sur_sa_propre_ligne() {
        let base = base(&[
            (1, "RA02-EN001", "Collector's Rare", -1_276_398_914),
            (2, "RA02-EN001", "Super Rare", -1_074_569_406),
            (3, "RA02-EN001", "Ultra Rare", 14_878_871),
        ]);
        let mut bases = HashMap::new();
        bases.insert("RA02".to_owned(), base);
        let lignes = vec![
            ligne(2, "RA02-FR001", "COL", 0),
            ligne(3, "RA02-FR001", "SR", 0),
            ligne(4, "RA02-FR001", "U", 0),
        ];
        let r = planifier(&lignes, &bases);
        assert!(r.refusees.is_empty(), "{:?}", r.refusees);
        let rowids: Vec<i64> = r.ecritures.iter().map(|e| e.rowid).collect();
        assert_eq!(rowids, vec![1, 2, 3], "trois raretés, trois lignes");
        assert!(r.fusions.is_empty(), "aucune ne se marche dessus");
    }

    /// Les trois causes de refus se distinguent — c'est tout l'intérêt du
    /// rapport.
    #[test]
    fn les_refus_nomment_la_dimension_qui_manque() {
        let mut bases = HashMap::new();
        bases.insert(
            "LDK2".to_owned(),
            base(&[(1, "LDK2-ENK01", "Common", 89_631_139)]),
        );
        let lignes = vec![
            ligne(2, "LDK2-FRT01", "U", 0),    // set_code jamais vu
            ligne(3, "LDK2-FRK01", "C", 1),    // rang inexistant
            ligne(4, "LDK2-FRK01", "SCR", 0),  // rareté inexistante
            ligne(5, "LDK2-FRK01", "ZZZZ", 0), // code hors référentiel
            ligne(6, "RA02-FR001", "C", 0),    // classeur absent
        ];
        let mut r = planifier(&lignes, &bases);
        r.refusees.sort_by_key(|x| x.numero);
        let causes: Vec<Refus> = r.refusees.iter().map(|x| x.refus).collect();
        assert_eq!(
            causes,
            vec![
                Refus::SetCodeAbsent,
                Refus::ArtworkAbsent,
                Refus::RareteAbsente,
                Refus::RareteInconnue,
                Refus::ClasseurAbsent,
            ]
        );
        assert!(r.ecritures.is_empty());
        assert_eq!(r.appariees(), 0);
        // Chaque cause a son propre libellé — le rapport ne dit pas cinq
        // fois la même chose.
        let libelles: HashSet<&str> = causes.iter().map(|c| c.libelle()).collect();
        assert_eq!(libelles.len(), 5);
    }

    /// Deux lignes CSV sur une même ligne de base : les quantités
    /// s'additionnent, et l'état de la seconde est gardé à part.
    #[test]
    fn une_fusion_additionne_et_garde_les_etats_differents() {
        let mut bases = HashMap::new();
        bases.insert(
            "VASM".to_owned(),
            base(&[(5, "VASM-EN003", "Super Rare", 1)]),
        );
        let mut premiere = ligne(248, "VASM-FR003", "SR", 0);
        premiere.qualite = "M".into();
        let mut seconde = ligne(249, "VASM-FR003", "SR", 0);
        seconde.edition = Some(Edition::Illimitee);
        seconde.qualite = "NM".into();

        let r = planifier(&[premiere, seconde], &bases);
        assert_eq!(r.ecritures.len(), 1);
        assert_eq!(r.ecritures[0].quantite, 2, "les deux exemplaires comptent");
        assert_eq!(
            r.ecritures[0].qualite, "M",
            "la première ligne du fichier gagne"
        );
        assert_eq!(r.ecritures[0].edition, Some(Edition::Premiere));
        // Depuis les exemplaires, la seconde ligne n'est plus perdue : elle
        // devient un exemplaire réglé à part.
        assert_eq!(
            r.ecritures[0].a_part,
            vec![(Etat::nouveau("NM", "unlimited"), 1)]
        );
        assert_eq!(r.fusions.len(), 1);
        assert_eq!(r.fusions[0].numeros, vec![248, 249]);
        assert!(!r.fusions[0].perte.langue);
        assert_eq!(r.fusions_avec_perte().count(), 0, "plus rien ne se perd");
    }

    /// Deux lignes rigoureusement identiques ne perdent rien : c'est
    /// simplement deux exemplaires.
    #[test]
    fn deux_lignes_identiques_ne_perdent_rien() {
        let mut bases = HashMap::new();
        bases.insert(
            "SDWD".to_owned(),
            base(&[(48, "SDWD-EN043", "Secret Rare", 7)]),
        );
        let r = planifier(
            &[
                ligne(406, "SDWD-FR043", "SCR", 0),
                ligne(407, "SDWD-FR043", "SCR", 0),
            ],
            &bases,
        );
        assert_eq!(r.ecritures[0].quantite, 2);
        assert_eq!(r.fusions.len(), 1, "la fusion est notée…");
        assert!(!r.fusions[0].perte.reelle(), "…mais elle n'efface rien");
        assert_eq!(r.fusions_avec_perte().count(), 0);
    }

    /// Un exemplaire anglais et un français de la même carte se fondent en
    /// une ligne — et l'import le dit, parce que la base n'a pas de langue.
    #[test]
    fn deux_langues_se_fondent_et_l_import_le_dit() {
        let mut bases = HashMap::new();
        bases.insert("SDWD".to_owned(), base(&[(1, "SDWD-EN001", "Common", 3)]));
        let mut anglaise = ligne(356, "SDWD-EN001", "C", 0);
        anglaise.langue = "Anglais (Monde)".into();
        let francaise = ligne(360, "SDWD-FR001", "C", 0);

        let r = planifier(&[anglaise, francaise], &bases);
        assert_eq!(r.ecritures.len(), 1);
        assert_eq!(r.ecritures[0].quantite, 2);
        assert_eq!(r.fusions.len(), 1);
        assert!(r.fusions[0].perte.langue, "la langue est ce qui se perd");
        assert!(r.ecritures[0].a_part.is_empty(), "même état, même édition");
    }

    /// Un code amputé de sa lettre de sous-jeu se reconnaît ; un code
    /// complet n'a rien à réparer.
    #[test]
    fn on_reconnait_un_code_prive_de_sa_lettre() {
        assert_eq!(sans_sous_jeu("LDK2-FR01"), Some(("LDK2-", "01")));
        assert_eq!(sans_sous_jeu("RA02-FR001"), Some(("RA02-", "001")));
        // Complets : rien à faire.
        assert_eq!(sans_sous_jeu("LDK2-ENJ01"), None);
        assert_eq!(sans_sous_jeu("MVP1-FRG05"), None);
        // Pas un code du tout.
        assert_eq!(sans_sous_jeu("PROMO"), None);
        assert_eq!(sans_sous_jeu("LDK2-FR"), None);
        assert_eq!(sans_sous_jeu(""), None);
    }

    /// La normalisation efface ce qui varie d'une source à l'autre, et rien
    /// de plus.
    #[test]
    fn la_normalisation_du_nom_efface_accents_et_ponctuation() {
        assert_eq!(normaliser_nom("Woùf es-tu ?"), normaliser_nom("Wouf es tu"));
        assert_eq!(
            normaliser_nom("Slifer, le Dragon Céleste"),
            "slifer le dragon celeste".replace(' ', "")
        );
        assert_eq!(normaliser_nom("Bœuf de Combat"), "boeufdecombat");
        assert_eq!(normaliser_nom("Âme Éternelle"), "ameeternelle");
        // Deux cartes qui ne sont pas la même ne se confondent pas.
        assert_ne!(
            normaliser_nom("Dragon Blanc aux Yeux Bleus"),
            normaliser_nom("Dragon Noir aux Yeux Rouges")
        );
    }

    /// Le cœur du repli : quatre sous-jeux, un même numéro, et le nom qui
    /// désigne lequel.
    #[test]
    fn le_nom_retrouve_la_lettre_de_sous_jeu() {
        let mut bases = HashMap::new();
        bases.insert(
            "LDK2".to_owned(),
            base_nommee(&[
                (
                    1,
                    "LDK2-ENJ01",
                    "Common",
                    74_677_422,
                    "Red-Eyes Black Dragon",
                    "Dragon Noir aux Yeux Rouges",
                ),
                (
                    44,
                    "LDK2-ENK01",
                    "Common",
                    89_631_139,
                    "Blue-Eyes White Dragon",
                    "Dragon Blanc aux Yeux Bleus",
                ),
                (
                    87,
                    "LDK2-ENS01",
                    "Ultra Rare",
                    10_000_020,
                    "Slifer the Sky Dragon",
                    "Slifer, le Dragon Céleste",
                ),
                (
                    93,
                    "LDK2-ENY01",
                    "Ultra Rare",
                    33_396_948,
                    "Exodia the Forbidden One",
                    "L'Incarné du Légendaire Éxodia",
                ),
            ]),
        );
        let attendu = [
            (75, "Dragon Noir aux Yeux Rouges", "C", 1_i64, "LDK2-ENJ01"),
            (118, "Dragon Blanc aux Yeux Bleus", "C", 44, "LDK2-ENK01"),
            (161, "Slifer, le Dragon Céleste", "U", 87, "LDK2-ENS01"),
            (167, "L'Incarné du Légendaire Éxodia", "U", 93, "LDK2-ENY01"),
        ];
        let lignes: Vec<Ligne> = attendu
            .iter()
            .map(|(numero, nom, rarete, _, _)| {
                let mut l = ligne(*numero, "LDK2-FR01", rarete, 0);
                l.nom = (*nom).to_owned();
                l
            })
            .collect();

        let r = planifier(&lignes, &bases);
        assert!(r.refusees.is_empty(), "{:?}", r.refusees);
        assert_eq!(r.ecritures.len(), 4, "quatre cartes, quatre lignes");
        assert!(r.fusions.is_empty(), "aucune ne se pose sur une autre");

        // Chaque ligne a bien trouvé SA carte, et le rapport le dit.
        let mut resolues = r.resolues.clone();
        resolues.sort();
        for ((numero, _, _, rowid, code), res) in attendu.iter().zip(&resolues) {
            assert_eq!(res.numero, *numero);
            assert_eq!(res.code_csv, "LDK2-FR01");
            assert_eq!(res.code_retenu, *code);
            assert!(
                r.ecritures.iter().any(|e| e.rowid == *rowid),
                "{code} devait aller sur le rowid {rowid}"
            );
        }
    }

    /// Un code complet n'emprunte jamais le repli : s'il ne trouve rien,
    /// c'est un refus, pas une occasion de deviner.
    #[test]
    fn un_code_complet_ne_passe_pas_par_le_repli() {
        let mut bases = HashMap::new();
        bases.insert(
            "LDK2".to_owned(),
            base_nommee(&[(
                1,
                "LDK2-ENJ01",
                "Common",
                74_677_422,
                "Red-Eyes Black Dragon",
                "Dragon Noir aux Yeux Rouges",
            )]),
        );
        let mut l = ligne(2, "LDK2-FRK01", "C", 0);
        l.nom = "Dragon Noir aux Yeux Rouges".to_owned();
        let r = planifier(&[l], &bases);
        assert_eq!(r.refusees.len(), 1, "le code dit K, la base dit J");
        assert_eq!(r.refusees[0].refus, Refus::SetCodeAbsent);
        assert!(r.resolues.is_empty());
    }

    /// Deux cartes homonymes sous le même numéro : le nom ne départage
    /// plus, et le repli refuse plutôt que de tirer au sort.
    #[test]
    fn deux_homonymes_sous_le_meme_numero_ne_se_devinent_pas() {
        let mut bases = HashMap::new();
        bases.insert(
            "LDK2".to_owned(),
            base_nommee(&[
                (
                    1,
                    "LDK2-ENJ22",
                    "Common",
                    1,
                    "Polymerization",
                    "Polymérisation",
                ),
                (
                    2,
                    "LDK2-ENK22",
                    "Common",
                    2,
                    "Polymerization",
                    "Polymérisation",
                ),
            ]),
        );
        let mut l = ligne(2, "LDK2-FR22", "C", 0);
        l.nom = "Polymérisation".to_owned();
        let r = planifier(&[l], &bases);
        assert_eq!(r.refusees.len(), 1, "deux candidats, aucun choix");
        assert_eq!(r.refusees[0].refus, Refus::SetCodeAbsent);
        assert!(r.resolues.is_empty(), "et rien n'a été deviné");
    }

    /// Le nom retrouve le tirage, mais l'artwork demandé n'existe pas : le
    /// refus parle de l'artwork, pas d'un code absent.
    #[test]
    fn le_repli_trouve_le_tirage_et_le_refus_nomme_la_vraie_cause() {
        let mut bases = HashMap::new();
        bases.insert(
            "LDK2".to_owned(),
            base_nommee(&[(
                44,
                "LDK2-ENK01",
                "Common",
                89_631_139,
                "Blue-Eyes White Dragon",
                "Dragon Blanc aux Yeux Bleus",
            )]),
        );
        let mut artwork = ligne(119, "LDK2-FR01", "C", 7);
        artwork.nom = "Dragon Blanc aux Yeux Bleus".to_owned();
        let mut rarete = ligne(120, "LDK2-FR01", "SCR", 0);
        rarete.nom = "Dragon Blanc aux Yeux Bleus".to_owned();

        let r = planifier(&[artwork, rarete], &bases);
        assert_eq!(r.refusees.len(), 2);
        assert_eq!(r.refusees[0].refus, Refus::ArtworkAbsent);
        assert_eq!(r.refusees[1].refus, Refus::RareteAbsente);
        assert!(r.resolues.is_empty(), "rien n'a été posé");
    }

    /// Le fichier réel amputé, contre la vraie base LDK2 : l'oracle du
    /// repli.
    ///
    /// Les 132 lignes LDK2 d'un export où la lettre de sous-jeu a été
    /// perdue. **Toutes** se rattachent par le nom, sans une seule
    /// ambiguïté — y compris les trois Dragons Blancs aux Yeux Bleus de
    /// `LDK2-ENK01`, que le rang par ligne distingue désormais alors qu'ils
    /// partagent une seule image.
    #[test]
    fn l_oracle_du_repli_sur_le_fichier_reel() {
        let csv = include_bytes!("../../../tests/fixtures/oracle/csv/ldk2_sans_sous_jeu.csv");
        let brut = include_str!("../../../tests/fixtures/oracle/csv/ldk2_base.json");
        #[derive(serde::Deserialize)]
        struct Brute {
            rowid: i64,
            set_code: String,
            rarity: String,
            card_image_id: Option<i64>,
            name: String,
            name_fr: Option<String>,
        }
        let brutes: HashMap<String, Vec<Brute>> = serde_json::from_str(brut).unwrap();
        let bases: HashMap<String, Vec<LigneBase>> = brutes
            .into_iter()
            .map(|(k, v)| {
                let lignes = v
                    .into_iter()
                    .map(|b| LigneBase {
                        rowid: b.rowid,
                        set_code: b.set_code,
                        rarity: b.rarity,
                        card_image_id: b.card_image_id,
                        name: b.name,
                        name_fr: b.name_fr,
                    })
                    .collect();
                (k, lignes)
            })
            .collect();

        let lecture = scanflip::lire_octets(csv).unwrap();
        assert_eq!(lecture.lignes.len(), 132);
        // Toutes les lignes du fichier sont amputées : c'est le cas à
        // traiter, pas un mélange.
        assert!(lecture
            .lignes
            .iter()
            .all(|l| sans_sous_jeu(&l.code).is_some()));

        let r = planifier(&lecture.lignes, &bases);
        assert!(r.refusees.is_empty(), "{:?}", r.refusees);
        assert_eq!(r.appariees(), 132);
        assert_eq!(r.resolues.len(), 132, "toutes passent par le repli");

        // Les quatre sous-jeux sont retrouvés, dans les proportions du set —
        // `K` en compte 43 pour 41 codes : trois exemplaires du Dragon Blanc.
        let mut lettres: BTreeMap<char, usize> = BTreeMap::new();
        for res in &r.resolues {
            let lettre = res.code_retenu.chars().nth(7).unwrap();
            *lettres.entry(lettre).or_default() += 1;
        }
        assert_eq!(
            lettres,
            [('J', 43), ('K', 43), ('S', 6), ('Y', 40)]
                .into_iter()
                .collect::<BTreeMap<char, usize>>()
        );

        // Aucune carte n'est allée sur la ligne d'une autre : 132 lignes de
        // CSV, 132 lignes de classeur distinctes.
        assert_eq!(r.ecritures.len(), 132);
        assert!(r.fusions.is_empty());
        let rowids: HashSet<i64> = r.ecritures.iter().map(|e| e.rowid).collect();
        assert_eq!(rowids.len(), 132);
    }

    /// La conversion vers l'anglais garde la lettre de sous-jeu et ne
    /// touche pas à ce qui n'est pas une langue.
    #[test]
    fn la_conversion_de_code_garde_la_lettre_de_sous_jeu() {
        assert_eq!(vers_anglais("LDK2-FRJ01"), "LDK2-ENJ01");
        assert_eq!(vers_anglais("LDK2-FRK41"), "LDK2-ENK41");
        assert_eq!(vers_anglais("MVP1-FRG05"), "MVP1-ENG05");
        assert_eq!(vers_anglais("SDLI-FR001"), "SDLI-EN001");
        // Déjà anglais : inchangé.
        assert_eq!(vers_anglais("RA02-EN001"), "RA02-EN001");
        // « DUEA-ENDE1 » : le suffixe « DE » n'est pas une langue ici, et
        // « EN » n'est pas dans la liste des langues sources.
        assert_eq!(vers_anglais("DUEA-ENDE1"), "DUEA-ENDE1");
        // Rien qui ressemble à un code : rendu tel quel.
        assert_eq!(vers_anglais("PROMO"), "PROMO");
        assert_eq!(vers_anglais("SANS-TIRETNUM"), "SANS-TIRETNUM");
        assert_eq!(vers_anglais(""), "");
    }

    /// La case « Rareté » n'est pas forcément un code.
    ///
    /// Scanflip accepte le libellé complet et sa forme courte ; un CSV
    /// retouché à la main les emploie. Refuser « Ultra Rare » quand on
    /// accepte « U » n'aurait aucun sens pour l'utilisateur.
    #[test]
    fn la_case_rarete_accepte_les_ecritures_de_scanflip() {
        for ecriture in ["U", "Ultra Rare", "Ultra", "ultra rare"] {
            let candidats = libelles_possibles(ecriture);
            assert!(
                candidats.iter().any(|c| c == "Ultra Rare"),
                "« {ecriture} » → {candidats:?}"
            );
        }
        // `STRB` n'est pas dans la table du Python : c'est le libellé
        // français de Scanflip qui sert de candidat.
        let strb = libelles_possibles("STRB");
        assert!(strb.iter().any(|c| c == "Starlight Blasonnée"), "{strb:?}");
    }

    /// Une case rareté reconnue trouve sa ligne, même écrite en toutes
    /// lettres.
    #[test]
    fn un_libelle_complet_apparie_comme_un_code() {
        let mut bases = HashMap::new();
        bases.insert(
            "RA02".to_owned(),
            base(&[(1, "RA02-EN001", "Ultra Rare", 1)]),
        );
        for ecriture in ["U", "Ultra Rare", "Ultra"] {
            let mut l = ligne(2, "RA02-FR001", ecriture, 0);
            l.rarete = ecriture.to_owned();
            let r = planifier(&[l], &bases);
            assert!(r.refusees.is_empty(), "« {ecriture} » : {:?}", r.refusees);
            assert_eq!(r.ecritures[0].rowid, 1);
        }
    }

    /// `C` désigne aussi les Short Print, que la canonisation garde
    /// distincts. Sans les alias, une carte Short Print exportée ne se
    /// réimporterait jamais.
    #[test]
    fn les_alias_du_referentiel_sont_des_candidats() {
        let candidats = libelles_possibles("C");
        assert_eq!(candidats.first().map(String::as_str), Some("Common"));
        assert!(candidats.iter().any(|c| c == "Short Print"));
        assert!(libelles_possibles("ZZZZ").is_empty());

        let mut bases = HashMap::new();
        bases.insert(
            "SDLI".to_owned(),
            base(&[(1, "SDLI-EN001", "Short Print", 1)]),
        );
        let r = planifier(&[ligne(2, "SDLI-FR001", "C", 0)], &bases);
        assert!(r.refusees.is_empty(), "{:?}", r.refusees);
        assert_eq!(r.ecritures[0].rowid, 1);
    }

    /// L'ordre du rapport ne dépend pas d'un hachage : deux exécutions
    /// rendent la même chose.
    #[test]
    fn le_rapport_est_ordonne_de_facon_stable() {
        let mut bases = HashMap::new();
        bases.insert(
            "RA02".to_owned(),
            base(&[
                (3, "RA02-EN003", "Common", 1),
                (1, "RA02-EN001", "Common", 1),
                (2, "RA02-EN002", "Common", 1),
            ]),
        );
        let lignes: Vec<Ligne> = (1..=3)
            .map(|i| ligne(i + 1, &format!("RA02-FR00{i}"), "C", 0))
            .collect();
        let a = planifier(&lignes, &bases);
        let b = planifier(&lignes, &bases);
        assert_eq!(a, b);
        assert_eq!(
            a.ecritures.iter().map(|e| e.rowid).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    /// Une installation jetable avec un classeur au vrai schéma.
    fn installation(lignes: &[(i64, &str, &str, i64)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (rowid, set_code, rarity, img) in lignes {
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, card_image_id, possessed, quantite)
                 VALUES (?1, 'Carte', ?2, ?3, ?4, 0, 0)",
                rusqlite::params![rowid, set_code, rarity, img],
            )
            .unwrap();
        }
        (tmp, paths)
    }

    /// L'écriture pose bien la quantité, l'état et l'édition sur la ligne
    /// visée — et sur elle seule.
    #[test]
    fn l_ecriture_touche_la_ligne_visee_et_pas_sa_voisine() {
        let (_tmp, paths) = installation(&[
            (1, "RA02-EN001", "Secret Rare", 10),
            (2, "RA02-EN001", "Super Rare", 20),
        ]);
        let rapport = Rapport {
            lues: 1,
            ecritures: vec![Ecriture {
                classeur: "RA02".into(),
                rowid: 1,
                quantite: 3,
                qualite: "NM".into(),
                edition: Some(Edition::Illimitee),
                a_part: Vec::new(),
            }],
            ..Rapport::default()
        };
        let bilan = appliquer(&paths, &rapport).unwrap();
        assert_eq!(bilan.ecrites, 1);
        assert_eq!(bilan.classeurs, vec!["RA02".to_owned()]);

        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let lu = |rowid: i64| -> (i64, i64, String, String) {
            conn.query_row(
                "SELECT possessed, quantite, COALESCE(qualite,''), COALESCE(edition,'')
                   FROM cards WHERE rowid = ?1",
                [rowid],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)),
            )
            .unwrap()
        };
        assert_eq!(lu(1), (1, 3, "NM".to_owned(), "unlimited".to_owned()));
        assert_eq!(
            lu(2),
            (0, 0, String::new(), String::new()),
            "la ligne voisine n'a pas bougé"
        );
    }

    /// Une quantité nulle ne rend pas la carte possédée.
    ///
    /// # Ce que cette garde évite
    ///
    /// Scanflip n'écrit pas de ligne à zéro, mais un CSV retouché à la main
    /// le peut. Sans le `quantite > 0`, la carte serait marquée possédée
    /// avec zéro exemplaire : elle apparaîtrait dans l'inventaire, serait
    /// comptée dans les totaux du classeur, et ressortirait à l'export —
    /// une possession fantôme que rien ne permettrait de faire disparaître
    /// depuis l'application.
    #[test]
    fn une_quantite_nulle_ne_rend_pas_la_carte_possedee() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", "Secret Rare", 10)]);
        let rapport = Rapport {
            lues: 1,
            ecritures: vec![Ecriture {
                classeur: "RA02".into(),
                rowid: 1,
                quantite: 0,
                qualite: String::new(),
                edition: None,
                a_part: Vec::new(),
            }],
            ..Rapport::default()
        };
        appliquer(&paths, &rapport).unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let possedee: i64 = conn
            .query_row("SELECT possessed FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(possedee, 0, "zéro exemplaire n'est pas une possession");
    }

    /// Le chemin complet : lire la base, apparier, écrire, relire.
    #[test]
    fn le_chemin_complet_passe_par_la_vraie_base() {
        let (_tmp, paths) = installation(&[
            (1, "RA02-EN001", "Secret Rare", 10),
            (2, "RA02-EN001", "Super Rare", 20),
        ]);
        let lignes = vec![ligne(2, "RA02-FR001", "SCR", 0)];
        let bases = bases_pour(&paths, &lignes);
        assert_eq!(bases.len(), 1, "la base du classeur a été lue");
        assert_eq!(bases["RA02"].len(), 2);

        let rapport = planifier(&lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert_eq!(rapport.ecritures[0].rowid, 1, "la Secret Rare, pas l'autre");
        appliquer(&paths, &rapport).unwrap();

        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let quantite: i64 = conn
            .query_row("SELECT quantite FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(quantite, 1);
    }

    /// Les classeurs absents se repèrent avant l'appariement, et le
    /// rapport les nomme après.
    #[test]
    fn les_classeurs_absents_se_reperent_avant_et_se_nomment_apres() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", "Secret Rare", 10)]);
        let lignes = vec![
            ligne(2, "RA02-FR001", "SCR", 0),
            ligne(3, "SDLI-FR001", "C", 0),
            ligne(4, "SDLI-FR002", "C", 0),
            ligne(5, "BLTR-FR001", "U", 0),
        ];
        // Avant : la liste vient du disque, sans rien apparier.
        assert_eq!(
            classeurs_manquants(&paths, &lignes),
            vec!["BLTR".to_owned(), "SDLI".to_owned()],
            "dédoublonnés et triés"
        );

        // Après : le rapport dit la même chose, depuis ses refus.
        let bases = bases_pour(&paths, &lignes);
        let rapport = planifier(&lignes, &bases);
        assert_eq!(
            rapport.classeurs_absents(),
            vec!["BLTR".to_owned(), "SDLI".to_owned()]
        );
        assert_eq!(rapport.refusees.len(), 3, "trois lignes, deux classeurs");
        assert_eq!(rapport.ecritures.len(), 1, "RA02 passe quand même");
    }

    /// Un classeur nommé par le CSV mais absent du disque n'est pas une
    /// erreur : ses lignes sont refusées, les autres passent.
    #[test]
    fn un_classeur_absent_du_disque_ne_fait_pas_tout_echouer() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", "Secret Rare", 10)]);
        let lignes = vec![
            ligne(2, "RA02-FR001", "SCR", 0),
            ligne(3, "SDLI-FR001", "C", 0),
        ];
        let bases = bases_pour(&paths, &lignes);
        assert_eq!(bases.len(), 1, "seul RA02 existe");
        let rapport = planifier(&lignes, &bases);
        assert_eq!(rapport.ecritures.len(), 1);
        assert_eq!(rapport.refusees.len(), 1);
        assert_eq!(rapport.refusees[0].refus, Refus::ClasseurAbsent);
    }

    /// Le fichier réel, contre les vraies bases : c'est l'oracle du lot.
    #[test]
    fn le_fichier_reel_s_apparie_sur_les_vraies_bases() {
        let csv = include_bytes!("../../../tests/fixtures/oracle/csv/scanflip_20260829.csv");
        let brut = include_str!("../../../tests/fixtures/oracle/csv/bases.json");
        #[derive(serde::Deserialize)]
        struct Brute {
            rowid: i64,
            set_code: String,
            rarity: String,
            card_image_id: Option<i64>,
            #[serde(default)]
            name: String,
            #[serde(default)]
            name_fr: Option<String>,
        }
        let brutes: HashMap<String, Vec<Brute>> = serde_json::from_str(brut).unwrap();
        let bases: HashMap<String, Vec<LigneBase>> = brutes
            .into_iter()
            .map(|(k, v)| {
                let lignes = v
                    .into_iter()
                    .map(|b| LigneBase {
                        rowid: b.rowid,
                        set_code: b.set_code,
                        rarity: b.rarity,
                        card_image_id: b.card_image_id,
                        name: b.name,
                        name_fr: b.name_fr,
                    })
                    .collect();
                (k, lignes)
            })
            .collect();

        let lecture = scanflip::lire_octets(csv).unwrap();
        let r = planifier(&lecture.lignes, &bases);

        assert_eq!(r.lues, 407);
        assert_eq!(r.appariees(), 402, "402 lignes trouvent leur place");
        assert_eq!(r.refusees.len(), 5);

        // Les cinq refus, nommés : trois jetons que les sources ne listent
        // pas, et deux artworks alternatifs non référencés.
        let mut causes: Vec<(String, Refus)> = r
            .refusees
            .iter()
            .map(|x| (x.code.clone(), x.refus))
            .collect();
        causes.sort();
        assert_eq!(
            causes,
            vec![
                ("LDK2-FRK01".to_owned(), Refus::ArtworkAbsent),
                ("LDK2-FRK01".to_owned(), Refus::ArtworkAbsent),
                ("LDK2-FRT01".to_owned(), Refus::SetCodeAbsent),
                ("LDK2-FRT02".to_owned(), Refus::SetCodeAbsent),
                ("LDK2-FRT03".to_owned(), Refus::SetCodeAbsent),
            ]
        );

        // Les 402 appariées se posent sur 393 lignes distinctes, dont neuf
        // reçoivent plusieurs exemplaires.
        assert_eq!(r.ecritures.len(), 393);
        assert_eq!(r.fusions.len(), 9);
        // Avant les exemplaires (2026-10-01) : huit fusions effaçaient une
        // langue, une édition ou un état. L'état et l'édition sont désormais
        // gardés à part ; ne restent perdues que les quatre langues.
        assert_eq!(
            r.fusions_avec_perte().count(),
            4,
            "quatre fusions mêlent deux langues — la seule perte qui reste"
        );
        let gardees: i64 = r
            .ecritures
            .iter()
            .flat_map(|e| e.a_part.iter().map(|(_, n)| *n))
            .sum();
        assert!(
            gardees > 0,
            "des exemplaires d'un autre état sont gardés à part"
        );
        for e in &r.ecritures {
            let a_part: i64 = e.a_part.iter().map(|(_, n)| *n).sum();
            assert!(
                a_part < e.quantite,
                "{}/{} : il reste l'état commun",
                e.classeur,
                e.rowid
            );
        }

        // Et le cœur de l'affaire : les quatorze lignes de RA02-FR001
        // tombent sur quatorze lignes de base distinctes.
        let chat: Vec<&Ecriture> = r
            .ecritures
            .iter()
            .filter(|e| e.classeur == "RA02" && e.quantite == 1)
            .collect();
        assert!(chat.len() >= 14);
        let lignes_chat = lecture
            .lignes
            .iter()
            .filter(|l| l.code == "RA02-FR001")
            .count();
        assert_eq!(lignes_chat, 14);
        let poses: HashSet<i64> = r
            .fusions
            .iter()
            .filter(|f| f.code == "RA02-FR001")
            .map(|f| f.rowid)
            .collect();
        assert!(
            poses.is_empty(),
            "aucune des quatorze ne se pose sur une ligne déjà prise"
        );
    }
}
