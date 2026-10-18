// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La galerie d'un set — `Set Card Galleries` — : pour chaque numéro, ses
//! scans.
//!
//! # Pourquoi — 2026-10-18
//!
//! Un nom de fichier Yugipedia ne dit pas son numéro :
//! `BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png` nomme une carte, un set, une
//! rareté et une illustration. Or `RA04` imprime Blue-Eyes White Dragon sous
//! deux numéros — `RA04-EN002` (une illustration en Quarter Century Secret
//! Rare) et `RA04-EN108` (deux) —, Dark Magician aussi (`EN001`, `EN106`).
//! La passe artworks distribuait donc les fichiers d'une carte à ses numéros
//! dans l'ordre de la liste : `EN002` recevait l'`AA`, qui est à `EN108`, et
//! `EN108` le scan nu, qui est à `EN002` — relevé sur la carte en main de
//! l'utilisateur.
//!
//! La galerie du set tranche : elle montre chaque numéro avec **son**
//! fichier, sous la rareté de son bloc.
//!
//! ```text
//! == Quarter Century Secret Rares ==
//! {{Set gallery|rarity=Quarter Century Secret Rare|
//! RA04-EN002; Blue-Eyes White Dragon; QCScR
//! RA04-EN108; Blue-Eyes White Dragon;; AA //description::(7th artwork)
//! RA04-EN108; Blue-Eyes White Dragon;; AA2 //description::(8th artwork)
//! }}
//! ```
//!
//! # Le format
//!
//! Un ou plusieurs blocs `{{Set gallery|…}}`. Une ligne **sans** point-virgule
//! porte des paramètres (`|rarity=Ultra Rare|`) qui valent pour les lignes
//! suivantes du bloc. Une ligne de carte : `NUMÉRO; Nom[; Rareté][; Suffixe]
//! [// options]` —
//!
//! - la **rareté** de la ligne, quand elle est là, l'emporte sur celle du
//!   bloc ; libellé complet (`Ultra Rare`, `LCKC`) ou abréviation (`QCScR`,
//!   `RA04`) ;
//! - le **suffixe** est celui du nom de fichier (`AA2`, `B`) : vide pour le
//!   fichier nu ;
//! - les options (`description::(…)`) ne servent qu'à la lecture humaine.
//!
//! L'**édition** est dans le titre de la page — `(TCG-EN-1E)` —, la région
//! aussi. Une page par édition : la passe ne s'intéresse qu'aux suffixes, que
//! les éditions partagent.
//!
//! # Ce que le module ne fait pas
//!
//! Il ne fabrique pas de nom de fichier : le nom de carte d'un fichier est
//! celui de la fiche **au jour où il a été versé** (`Garden of Talistones`
//! quand la base dit `Protective Stone Gardenscaping`), et son extension
//! n'est pas écrite. Il dit, pour un numéro et une rareté, quels suffixes lui
//! appartiennent ; c'est aux fichiers réellement listés de fournir le reste.

use std::collections::{BTreeMap, BTreeSet};

use super::artwork::{abbr_rarete, cle_comparaison, ABBR_RARETE};
use super::structure::{regions_candidates, PAGES_COMPLEMENTAIRES};
use crate::error::Result;
use crate::http::ClientHttp;

/// L'espace de noms MediaWiki des galeries de sets.
pub const ESPACE_GALERIES: u32 = 3024;

/// Le préfixe des titres de galerie.
pub const PREFIXE_TITRE: &str = "Set Card Galleries:";

/// Les éditions d'une page, dans l'ordre où la passe les préfère : la même
/// que pour les fichiers (`1E`, sans édition, `UE`, `LE`).
pub const EDITIONS_PREFEREES: [&str; 4] = ["-1E", "", "-UE", "-LE"];

const BALISE: &str = "{{Set gallery";

/// Une carte de la galerie : un numéro, une rareté, un fichier.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntreeGalerie {
    /// Le numéro, en majuscules (`RA04-EN108`).
    pub numero: String,
    /// Le nom de la carte tel que la page l'écrit (`Dark Magician (Arkana)`).
    pub nom: String,
    /// La rareté telle qu'écrite — celle de la ligne, à défaut celle du bloc.
    pub rarete: String,
    /// Son abréviation Yugipedia, en majuscules (`QCSCR`) ; vide si inconnue.
    pub abbr: String,
    /// Le suffixe du fichier, en majuscules (`AA2`) ; vide pour le fichier
    /// nu.
    pub suffixe: String,
    /// La description de la ligne (`(8th artwork)`), vide sinon.
    pub description: String,
}

impl EntreeGalerie {
    /// Les clés de carte sous lesquelles un fichier peut nommer l'entrée :
    /// son nom, et son nom sans la précision finale entre parenthèses — la
    /// page écrit `Dark Magician (Arkana)`, le fichier `DarkMagician-…`.
    ///
    /// ```
    /// use ygo_sources::yugipedia::galerie::EntreeGalerie;
    /// let e = EntreeGalerie { nom: "Dark Magician (Arkana)".into(), ..Default::default() };
    /// assert_eq!(
    ///     e.cles().into_iter().collect::<Vec<_>>(),
    ///     vec!["darkmagician".to_owned(), "darkmagicianarkana".to_owned()]
    /// );
    /// ```
    #[must_use]
    pub fn cles(&self) -> BTreeSet<String> {
        let mut cles = BTreeSet::new();
        cles.insert(cle_comparaison(&self.nom));
        if let Some(sans) = sans_precision(&self.nom) {
            cles.insert(cle_comparaison(sans));
        }
        cles.retain(|c| !c.is_empty());
        cles
    }
}

/// Le nom sans sa précision finale : `Dark Magician (Arkana)` →
/// `Dark Magician`. `None` s'il n'en a pas.
fn sans_precision(nom: &str) -> Option<&str> {
    let nom = nom.trim();
    if !nom.ends_with(')') {
        return None;
    }
    let ouverture = nom.rfind(" (")?;
    nom.get(..ouverture)
        .map(str::trim)
        .filter(|n| !n.is_empty())
}

/// La galerie d'un set : ses pages, et leurs cartes dans l'ordre.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Galerie {
    /// Les pages lues — la page du set, puis celles de ses paquets bonus.
    pub titres: Vec<String>,
    /// Les cartes, dans l'ordre des pages.
    pub entrees: Vec<EntreeGalerie>,
}

impl Galerie {
    /// Une galerie qui ne dit rien.
    #[must_use]
    pub fn est_vide(&self) -> bool {
        self.entrees.is_empty()
    }

    /// La galerie montre-t-elle ce numéro dans cette rareté ?
    #[must_use]
    pub fn decrit(&self, numero: &str, abbr: &str) -> bool {
        self.entrees.iter().any(|e| {
            e.numero.eq_ignore_ascii_case(numero.trim()) && e.abbr.eq_ignore_ascii_case(abbr)
        })
    }

    /// Les suffixes de ce numéro dans cette rareté, dans l'ordre de la page —
    /// un par illustration : `RA04-EN108` en Quarter Century Secret Rare
    /// donne `AA`, `AA2`.
    #[must_use]
    pub fn suffixes(&self, numero: &str, abbr: &str) -> Vec<String> {
        self.entrees
            .iter()
            .filter(|e| {
                e.numero.eq_ignore_ascii_case(numero.trim()) && e.abbr.eq_ignore_ascii_case(abbr)
            })
            .map(|e| e.suffixe.clone())
            .collect()
    }

    /// Les numéros qui portent cette illustration d'une carte : ceux dont une
    /// entrée a cette rareté, ce suffixe, et l'une des clés de carte données.
    #[must_use]
    pub fn numeros(&self, cles: &BTreeSet<String>, abbr: &str, suffixe: &str) -> BTreeSet<String> {
        self.entrees
            .iter()
            .filter(|e| {
                e.abbr.eq_ignore_ascii_case(abbr)
                    && e.suffixe.eq_ignore_ascii_case(suffixe)
                    && e.cles().iter().any(|c| cles.contains(c))
            })
            .map(|e| e.numero.clone())
            .collect()
    }

    /// Chaque numéro décrit, avec ses suffixes par rareté — la vue d'un
    /// classeur entier.
    #[must_use]
    pub fn par_numero(&self) -> BTreeMap<(String, String), Vec<String>> {
        let mut index: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for e in &self.entrees {
            if e.abbr.is_empty() {
                continue;
            }
            index
                .entry((e.numero.clone(), e.abbr.clone()))
                .or_default()
                .push(e.suffixe.clone());
        }
        index
    }
}

/// L'abréviation d'une rareté telle qu'une galerie l'écrit — libellé complet
/// (`Quarter Century Secret Rare`) ou abréviation (`QCScR`) —, en
/// majuscules ; vide si inconnue.
///
/// ```
/// use ygo_sources::yugipedia::galerie::abbr_de;
/// assert_eq!(abbr_de("Quarter Century Secret Rare"), "QCSCR");
/// assert_eq!(abbr_de("QCScR"), "QCSCR");
/// assert_eq!(abbr_de("Normal Rare"), "NR");
/// assert_eq!(abbr_de("ultra rare "), "UR");
/// assert_eq!(abbr_de(""), "");
/// assert_eq!(abbr_de("Rareté martienne"), "");
/// ```
#[must_use]
pub fn abbr_de(rarete: &str) -> String {
    let texte = rarete.trim();
    if texte.is_empty() {
        return String::new();
    }
    let par_libelle = abbr_rarete(texte);
    if !par_libelle.is_empty() {
        return par_libelle.to_ascii_uppercase();
    }
    ABBR_RARETE
        .iter()
        .find(|(_, abbr)| !abbr.is_empty() && abbr.eq_ignore_ascii_case(texte))
        .map(|(_, abbr)| abbr.to_ascii_uppercase())
        .unwrap_or_default()
}

/// Un suffixe de fichier tel qu'une galerie l'écrit : lettres, chiffres et
/// tirets seulement — sinon ce n'en est pas un, et la ligne vaut pour le
/// fichier nu.
fn suffixe_de(champ: &str) -> String {
    let t = champ.trim();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        t.to_ascii_uppercase()
    } else {
        String::new()
    }
}

/// Le contenu de `description::(…)` dans des options de ligne, parenthèses
/// comprises.
fn description(options: &str) -> String {
    let Some(debut) = options.find("description::") else {
        return String::new();
    };
    let reste = options.get(debut + "description::".len()..).unwrap_or("");
    match reste.find(')') {
        Some(fin) => reste.get(..=fin).unwrap_or("").trim().to_owned(),
        None => reste.trim().to_owned(),
    }
}

/// Analyse le wikitext d'une page de galerie.
///
/// ```
/// use ygo_sources::yugipedia::galerie::parse_galerie;
/// let e = parse_galerie(
///     "{{Set gallery|rarity=Quarter Century Secret Rare|\n\
///      RA04-EN002; Blue-Eyes White Dragon; QCScR\n\
///      RA04-EN108; Blue-Eyes White Dragon;; AA2 //description::(8th artwork)\n}}",
/// );
/// assert_eq!(e.len(), 2);
/// assert_eq!((e[0].numero.as_str(), e[0].abbr.as_str(), e[0].suffixe.as_str()), ("RA04-EN002", "QCSCR", ""));
/// assert_eq!((e[1].abbr.as_str(), e[1].suffixe.as_str()), ("QCSCR", "AA2"));
/// assert_eq!(e[1].description, "(8th artwork)");
/// ```
#[must_use]
pub fn parse_galerie(wikitext: &str) -> Vec<EntreeGalerie> {
    let mut entrees = Vec::new();
    for corps in super::extraire_blocs_de(wikitext, BALISE) {
        let mut rarete_du_bloc = String::new();
        for ligne in corps.split('\n') {
            let ligne = ligne.trim();
            if ligne.is_empty() {
                continue;
            }
            if !ligne.contains(';') {
                for jeton in ligne.split('|') {
                    if let Some((cle, valeur)) = jeton.split_once('=') {
                        if cle.trim().eq_ignore_ascii_case("rarity") {
                            valeur.trim().clone_into(&mut rarete_du_bloc);
                        }
                    }
                }
                continue;
            }
            let entree = ligne.trim_start_matches('|').trim();
            let (principal, options) = match entree.split_once("//") {
                Some((avant, apres)) => (avant, apres),
                None => (entree, ""),
            };
            let champs: Vec<&str> = principal.split(';').map(str::trim).collect();
            let numero = champs.first().copied().unwrap_or_default().to_uppercase();
            if numero.is_empty() || !numero.contains('-') {
                continue;
            }
            let rarete = match champs.get(2) {
                Some(r) if !r.is_empty() => (*r).to_owned(),
                _ => rarete_du_bloc.clone(),
            };
            entrees.push(EntreeGalerie {
                numero,
                nom: champs
                    .get(1)
                    .map(|n| super::nettoyer(n))
                    .unwrap_or_default(),
                abbr: abbr_de(&rarete),
                rarete,
                suffixe: champs.get(3).map_or_else(String::new, |s| suffixe_de(s)),
                description: description(options),
            });
        }
    }
    entrees
}

/// L'édition qu'un titre de galerie annonce : `(TCG-EN-1E)` → `1E`, `""`
/// sans édition.
///
/// ```
/// use ygo_sources::yugipedia::galerie::edition_du_titre;
/// assert_eq!(edition_du_titre("Set Card Galleries:Quarter Century Stampede (TCG-EN-1E)"), "1E");
/// assert_eq!(edition_du_titre("Set Card Galleries:Chaos Origins (OCG-JP)"), "");
/// ```
#[must_use]
pub fn edition_du_titre(titre: &str) -> String {
    let Some((_, fin)) = titre.rsplit_once('(') else {
        return String::new();
    };
    let interieur = fin.trim_end_matches(')');
    match interieur.rsplit('-').next() {
        Some(e) if ["1E", "UE", "LE"].contains(&e) => e.to_owned(),
        _ => String::new(),
    }
}

/// Les pages de galerie à lire pour un set, parmi les titres existants : la
/// page du set dans la région de la langue — première édition d'abord, puis
/// sans édition, `UE`, `LE` —, puis celles de ses paquets bonus dans la même
/// région et la même édition (`CORI-JPS10`, « +1 Expansion Pack »).
///
/// Le titre doit être **exactement** celui du set : « Legendary Collection
/// Kaiba » n'est pas « Legendary Collection Kaiba Mega Pack ». Une autre
/// région n'est jamais prise — ses numéros ne sont pas ceux du classeur.
///
/// ```
/// use ygo_sources::yugipedia::galerie::choisir_titres;
/// let titres: Vec<String> = [
///     "Set Card Galleries:Chaos Origins (OCG-JP)",
///     "Set Card Galleries:Chaos Origins (TCG-EN-1E)",
///     "Set Card Galleries:Chaos Origins +1 Expansion Pack (OCG-JP)",
/// ].map(String::from).to_vec();
/// assert_eq!(
///     choisir_titres(&titres, "Chaos Origins", "JP"),
///     vec![
///         "Set Card Galleries:Chaos Origins (OCG-JP)".to_owned(),
///         "Set Card Galleries:Chaos Origins +1 Expansion Pack (OCG-JP)".to_owned(),
///     ]
/// );
/// assert_eq!(
///     choisir_titres(&titres, "Chaos Origins", "EN"),
///     vec!["Set Card Galleries:Chaos Origins (TCG-EN-1E)".to_owned()]
/// );
/// assert!(choisir_titres(&titres, "Chaos Origins", "FR").is_empty());
/// ```
#[must_use]
pub fn choisir_titres(titres: &[String], nom_set: &str, langue: &str) -> Vec<String> {
    let nom = nom_set.trim();
    if nom.is_empty() {
        return Vec::new();
    }
    let langue = match langue.trim() {
        "" => "EN".to_owned(),
        l => l.to_ascii_uppercase(),
    };
    let marqueur = format!("-{langue}");
    // Les régions de la Set list, moins ses replis sur une autre langue :
    // les numéros d'une autre région ne sont pas ceux du classeur.
    for region in regions_candidates(&langue)
        .into_iter()
        .filter(|r| r.ends_with(&marqueur))
    {
        for edition in EDITIONS_PREFEREES {
            let principal = format!("{PREFIXE_TITRE}{nom} ({region}{edition})");
            if !titres.contains(&principal) {
                continue;
            }
            let mut choisis = vec![principal];
            choisis.extend(
                PAGES_COMPLEMENTAIRES
                    .iter()
                    .map(|s| format!("{PREFIXE_TITRE}{nom}{s} ({region}{edition})"))
                    .filter(|t| titres.contains(t)),
            );
            return choisis;
        }
    }
    Vec::new()
}

#[derive(serde::Deserialize)]
struct ReponsePages {
    #[serde(default)]
    query: Option<RequetePages>,
}

#[derive(serde::Deserialize)]
struct RequetePages {
    #[serde(default)]
    allpages: Vec<PageListee>,
}

#[derive(serde::Deserialize)]
struct PageListee {
    #[serde(default)]
    title: String,
}

/// Les titres de galerie qui commencent par le nom du set — une requête
/// `allpages`, gardée 30 jours par le cache ([`crate::cache::duree_de_cache`]).
///
/// # Errors
///
/// Rend l'erreur du réseau.
pub async fn titres_galerie(client: &ClientHttp, nom_set: &str) -> Result<Vec<String>> {
    let nom = nom_set.trim();
    if nom.is_empty() {
        return Ok(Vec::new());
    }
    let url = super::url_api(&[
        ("action", "query"),
        ("list", "allpages"),
        ("apnamespace", &ESPACE_GALERIES.to_string()),
        ("apprefix", nom),
        ("aplimit", "50"),
    ]);
    let reponse: ReponsePages = client.get_json(&url).await?;
    Ok(reponse
        .query
        .map(|q| q.allpages.into_iter().map(|p| p.title).collect())
        .unwrap_or_default())
}

/// La galerie d'un set, dans la langue de ses numéros — vide si Yugipedia
/// n'en a pas.
///
/// Deux requêtes la première fois (la liste des pages, puis la page), une de
/// plus par paquet bonus ; ensuite plus aucune pendant 30 jours.
///
/// # Errors
///
/// Rend l'erreur du réseau.
pub async fn lire(client: &ClientHttp, nom_set: &str, langue: &str) -> Result<Galerie> {
    let titres = choisir_titres(&titres_galerie(client, nom_set).await?, nom_set, langue);
    let mut galerie = Galerie::default();
    for titre in titres {
        if let Some(wikitext) = super::artwork::wikitext_page(client, &titre).await? {
            galerie.entrees.extend(parse_galerie(&wikitext));
            galerie.titres.push(titre);
        }
    }
    if !galerie.est_vide() {
        tracing::info!(
            set = nom_set,
            pages = galerie.titres.len(),
            cartes = galerie.entrees.len(),
            "galerie Yugipedia lue"
        );
    }
    Ok(galerie)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Un extrait de `Set Card Galleries:Quarter Century Stampede (TCG-EN-1E)`,
    /// lignes recopiées telles quelles le 2026-10-18 : l'en-tête, deux blocs
    /// entiers en tête et en fin, et les numéros de Dark Magician et de
    /// Blue-Eyes White Dragon.
    const RA04: &str =
        include_str!("../../../../tests/fixtures/net/yugipedia/galerie_RA04-EN-1E_extrait.txt");

    /// Idem pour `Legendary Collection Kaiba Mega Pack (TCG-EN-1E)` : un bloc
    /// sans rareté, la rareté et le suffixe sur chaque ligne.
    const LCKC: &str =
        include_str!("../../../../tests/fixtures/net/yugipedia/galerie_LCKC-EN-1E_extrait.txt");

    fn suffixes(g: &Galerie, numero: &str, abbr: &str) -> Vec<String> {
        g.suffixes(numero, abbr)
    }

    fn galerie(texte: &str) -> Galerie {
        Galerie {
            titres: vec!["essai".into()],
            entrees: parse_galerie(texte),
        }
    }

    /// Le cas de l'utilisateur : chaque numéro a ses fichiers, et un même
    /// fichier n'appartient qu'à un numéro.
    #[test]
    fn chaque_numero_de_ra04_a_ses_fichiers() {
        let g = galerie(RA04);
        assert_eq!(suffixes(&g, "RA04-EN002", "QCSCR"), [""], "le scan nu");
        assert_eq!(suffixes(&g, "RA04-EN108", "QCSCR"), ["AA", "AA2"]);
        assert_eq!(suffixes(&g, "RA04-EN108", "PLSCR"), ["", "AA"]);
        assert_eq!(suffixes(&g, "RA04-EN001", "QCSCR"), [""]);
        assert_eq!(
            suffixes(&g, "RA04-EN106", "QCSCR"),
            ["AA", "AA2", "AA3", "AA4", "AA5"]
        );
        assert_eq!(
            suffixes(&g, "RA04-EN106", "PLSCR"),
            ["", "AA2", "AA", "AA3", "AA4"]
        );
        assert!(
            !g.decrit("RA04-EN001", "PLSCR"),
            "pas de Platinum pour EN001"
        );
        assert!(g.decrit("ra04-en002", "qcscr"), "casse indifférente");

        // Le scan nu de Blue-Eyes en QCScR est à EN002, et à lui seul.
        let blue_eyes: BTreeSet<String> = ["blueeyeswhitedragon".to_owned()].into();
        assert_eq!(
            g.numeros(&blue_eyes, "QCSCR", "")
                .into_iter()
                .collect::<Vec<_>>(),
            ["RA04-EN002"]
        );
        assert_eq!(
            g.numeros(&blue_eyes, "QCSCR", "AA")
                .into_iter()
                .collect::<Vec<_>>(),
            ["RA04-EN108"]
        );
        // Le Dark Magician d'Arkana se cherche aussi sous « Dark Magician ».
        let dm: BTreeSet<String> = ["darkmagician".to_owned()].into();
        assert_eq!(
            g.numeros(&dm, "QCSCR", "AA")
                .into_iter()
                .collect::<Vec<_>>(),
            ["RA04-EN106"]
        );
        let arkana = g
            .entrees
            .iter()
            .find(|e| e.numero == "RA04-EN106" && e.abbr == "QCSCR" && e.suffixe == "AA")
            .unwrap();
        assert_eq!(arkana.nom, "Dark Magician (Arkana)");
        assert_eq!(arkana.description, "(Arkana)");
    }

    /// Les blocs ordinaires : la rareté du bloc vaut pour toutes ses lignes.
    #[test]
    fn la_rarete_du_bloc_vaut_pour_ses_lignes() {
        let g = galerie(RA04);
        let piper: Vec<&EntreeGalerie> = g
            .entrees
            .iter()
            .filter(|e| e.numero == "RA04-EN005")
            .collect();
        assert!(piper
            .iter()
            .any(|e| e.abbr == "SR" && e.rarete == "Super Rare"));
        assert!(piper.iter().all(|e| e.suffixe.is_empty()));
        let par_numero = g.par_numero();
        assert_eq!(
            par_numero.get(&("RA04-EN108".to_owned(), "QCSCR".to_owned())),
            Some(&vec!["AA".to_owned(), "AA2".to_owned()])
        );
    }

    /// Un bloc sans rareté : chaque ligne porte la sienne, et son suffixe,
    /// espaces de fin compris.
    #[test]
    fn la_rarete_et_le_suffixe_peuvent_etre_sur_la_ligne() {
        let g = galerie(LCKC);
        assert_eq!(suffixes(&g, "LCKC-EN001", "UR"), ["", "B", "C", "D"]);
        assert_eq!(suffixes(&g, "LCKC-EN046", "UR"), ["", "B"]);
        assert_eq!(suffixes(&g, "LCKC-EN002", "UR"), [""]);
    }

    /// Ce qui n'est pas une carte n'entre pas : paramètres, en-tête, lignes
    /// sans numéro, suffixe illisible.
    #[test]
    fn ce_qui_n_est_pas_une_carte_n_entre_pas() {
        let e = parse_galerie(
            "{{Set page header}}\n{{Set gallery|rarity=Common|columns=4|\n\
             ; sans numéro\nPROMO; pas de tiret\n\
             X-EN001; Carte; ; AA 2\n\
             X-EN002; [[Dark Magician|Magicien]]; Normal Rare; B\n}}\n{{Set gallery|",
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].suffixe, "", "« AA 2 » n'est pas un suffixe");
        assert_eq!(e[0].abbr, "C", "la rareté du bloc");
        assert_eq!(e[1].nom, "Magicien", "les liens se déroulent");
        assert_eq!((e[1].abbr.as_str(), e[1].suffixe.as_str()), ("NR", "B"));
        assert!(parse_galerie("").is_empty());
        assert!(
            parse_galerie("{{Set gallery").is_empty(),
            "et pas de panique"
        );
    }

    /// Le titre : la région de la langue, l'édition préférée, le nom exact.
    #[test]
    fn le_titre_est_celui_du_set_dans_la_region_et_l_edition_preferees() {
        let titres = |t: &[&str]| t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let ra04 = titres(&[
            "Set Card Galleries:Quarter Century Stampede (TCG-DE-1E)",
            "Set Card Galleries:Quarter Century Stampede (TCG-EN-1E)",
            "Set Card Galleries:Quarter Century Stampede (TCG-FR-1E)",
        ]);
        assert_eq!(
            choisir_titres(&ra04, "Quarter Century Stampede", "EN"),
            ["Set Card Galleries:Quarter Century Stampede (TCG-EN-1E)"]
        );
        let kaiba = titres(&[
            "Set Card Galleries:Legendary Collection Kaiba (TCG-EN-LE)",
            "Set Card Galleries:Legendary Collection Kaiba Mega Pack (TCG-EN-UE)",
            "Set Card Galleries:Legendary Collection Kaiba Mega Pack (TCG-EN-1E)",
        ]);
        assert_eq!(
            choisir_titres(&kaiba, "Legendary Collection Kaiba Mega Pack", "EN"),
            ["Set Card Galleries:Legendary Collection Kaiba Mega Pack (TCG-EN-1E)"],
            "1E avant UE"
        );
        assert_eq!(
            choisir_titres(&kaiba, "Legendary Collection Kaiba", "EN"),
            ["Set Card Galleries:Legendary Collection Kaiba (TCG-EN-LE)"],
            "le nom exact, pas le Mega Pack"
        );
        let sans = titres(&[
            "Set Card Galleries:X (TCG-EN-UE)",
            "Set Card Galleries:X (TCG-EN)",
        ]);
        assert_eq!(
            choisir_titres(&sans, "X", "EN"),
            ["Set Card Galleries:X (TCG-EN)"]
        );
        assert!(choisir_titres(&sans, "", "EN").is_empty());
        assert_eq!(edition_du_titre("Set Card Galleries:X (TCG-EN-LE)"), "LE");
        assert_eq!(edition_du_titre("sans parenthèse"), "");
    }
}
