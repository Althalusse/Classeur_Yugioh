// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Rendre la collection au format Scanflip.
//!
//! Portage de `module/export/export_collection.py`. L'export est le miroir
//! de [`crate::import`] : il emploie **le même calcul de rang d'artwork**,
//! par `(set_code, rareté)`, sans quoi un aller-retour déplacerait les
//! cartes d'une ligne à l'autre.
//!
//! # Le libellé de langue du Python n'est pas celui de Scanflip
//!
//! La V1.0.4 écrit `English`. L'export réel de Scanflip écrit **`Anglais
//! (Monde)`** — ses libellés sont en français quelle que soit la langue de
//! la carte. Le Python promet pourtant un « round-trip exact » dans sa
//! propre docstring : il ne l'a jamais tenu pour les cartes anglaises.
//! Corrigé ici, faute de quoi Scanflip relirait ses propres lignes avec un
//! libellé qu'il n'écrit pas.
//!
//! # Ce qui n'est pas exportable
//!
//! La base n'a **pas de langue par exemplaire**. Un utilisateur qui possède
//! la même carte en anglais et en français a une seule ligne, de quantité
//! deux : l'export les rend dans la langue demandée pour le fichier. C'est
//! la perte que l'import signale déjà à l'entrée, et elle est symétrique.
//!
//! # Deux formats : « Scanflip » et « Application » — 2026-10-18
//!
//! L'export servait deux maîtres à la fois : Scanflip, qui ne connaît pas
//! l'OCG et attend des codes dans la langue de la carte, et l'application
//! elle-même, qui relit le fichier dans une autre installation (sauvegarde,
//! transfert vers une nouvelle version). Le compromis — classeurs OCG inclus,
//! codes TCG traduits — ne contentait ni l'un ni l'autre : Scanflip recevait
//! des lignes qu'il rejette, et l'autre installation des codes qu'elle n'a
//! pas (`CORI-ENS10` d'un classeur japonais, exporté en `CORI-FRS10`).
//! L'utilisateur choisit désormais ([`Format`]) :
//!
//! - **Scanflip** : ses onze colonnes, codes traduits dans la langue du
//!   fichier, et **sans les classeurs OCG** ([`classeurs_du_format`]) —
//!   [`Bilan::ecartes`] les nomme.
//! - **Application** : tous les classeurs, **codes tels qu'en base**, et les
//!   colonnes qui désignent une ligne sans deviner
//!   ([`crate::scanflip::Complement`] : rareté exacte, nom anglais,
//!   illustration, identifiant d'image, Overframe). La colonne `Langue` garde
//!   la langue de la collection (celle de l'en-tête, celle de sa région pour
//!   un tirage OCG) : la base ne connaît pas la langue d'un exemplaire, et un
//!   code en `-EN` n'en dit rien.
//!
//! # Les classeurs OCG — 2026-10-11
//!
//! Un tirage OCG garde son code natif (`LOCR-JP001`) et reçoit le libellé de
//! sa langue ([`libelle_ocg`]), les autres celui du fichier. Scanflip ne
//! connaissant pas l'OCG, ses classeurs ne sortent plus qu'au format
//! « Application » ; [`Bilan::ocg`] les nomme.
//!
//! # L'état inconnu reste vide
//!
//! Une carte saisie au clic dans le classeur n'a pas d'état : elle n'est
//! jamais passée par un import. Sur l'installation réelle, 176 lignes de
//! `RA05` sont dans ce cas. L'export **n'invente rien** — écrire « NM » par
//! défaut affirmerait un état de conservation que personne n'a constaté.
//! Il les compte et le dit ; l'état se renseigne en masse depuis
//! l'inventaire ([`crate::inventaire::definir_qualite`]).

use std::collections::HashMap;

use ygo_core::config::a_suffixe_ocg;
use ygo_core::paths::Paths;
use ygo_core::rarity::{self, reference};

use crate::error::{AppError, Result};
use crate::import;
use crate::scanflip::{Complement, Edition, Sortie};

/// Ce qu'un export vise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Scanflip : onze colonnes, codes dans la langue du fichier, classeurs
    /// OCG écartés.
    #[default]
    Scanflip,
    /// Une autre installation de l'application : tous les classeurs, codes
    /// tels qu'en base, colonnes d'identité en plus.
    Application,
}

impl Format {
    /// Le nom montré à l'utilisateur.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Scanflip => "Scanflip",
            Self::Application => "Application",
        }
    }

    /// Les deux formats, dans l'ordre où l'écran les propose.
    #[must_use]
    pub fn tous() -> [Self; 2] {
        [Self::Scanflip, Self::Application]
    }
}

/// La langue d'un fichier exporté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Langue {
    /// Codes carte en `-FR`, libellé « Français (France) ».
    #[default]
    Francais,
    /// Codes carte inchangés, libellé « Anglais (Monde) ».
    Anglais,
}

impl Langue {
    /// Le libellé que Scanflip écrit dans la colonne `Langue`.
    ///
    /// Les deux sont en français : ce sont les libellés relevés dans un
    /// export réel, pas une traduction de notre cru.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Francais => "Français (France)",
            Self::Anglais => "Anglais (Monde)",
        }
    }

    /// Le code de langue à écrire dans un `set_code`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Francais => "FR",
            Self::Anglais => "EN",
        }
    }
}

/// Une ligne possédée, telle que la base la donne.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Possedee {
    /// Identité de la ligne.
    pub rowid: i64,
    /// Le classeur d'où elle vient.
    pub classeur: String,
    /// Code du tirage.
    pub set_code: String,
    /// Libellé de rareté en base.
    pub rarity: String,
    /// Identifiant d'image — pour le rang d'artwork.
    pub card_image_id: Option<i64>,
    /// Nom anglais.
    pub name: String,
    /// Nom français, s'il est connu.
    pub name_fr: String,
    /// Nombre d'exemplaires.
    pub quantite: i64,
    /// État.
    pub qualite: String,
    /// Édition stockée (`1st`, `unlimited`, `limited`).
    pub edition: String,
    /// Adresse de l'image (`card_image_url`) — l'illustration de l'export
    /// « Application ».
    pub image: String,
    /// Tirage à cadre étendu (`extended_art`).
    pub overframe: bool,
}

/// Traduit un code carte vers la langue de l'export.
///
/// Les tirages **OCG** ne sont pas traduits : `LOCH-JP001` n'a jamais existé
/// en `-FR`, et fabriquer ce code produirait un identifiant qui ne désigne
/// rien. Un classeur japonais s'exporte donc avec ses codes natifs, quelle
/// que soit la langue demandée.
///
/// ```
/// use ygo_app::export::{traduire_code, Langue};
/// assert_eq!(traduire_code("RA02-EN001", Langue::Francais), "RA02-FR001");
/// assert_eq!(traduire_code("LDK2-ENJ01", Langue::Francais), "LDK2-FRJ01");
/// assert_eq!(traduire_code("LOCH-JP001", Langue::Francais), "LOCH-JP001");
/// ```
#[must_use]
pub fn traduire_code(code: &str, langue: Langue) -> String {
    if code.is_empty() || a_suffixe_ocg(code) {
        return code.to_owned();
    }
    let Some(tiret) = code.rfind('-') else {
        return code.to_owned();
    };
    let (prefixe, reste) = code.split_at(tiret + 1);
    if reste.len() < 3 {
        return code.to_owned();
    }
    let (deux, suffixe) = reste.split_at(2);
    if !deux.chars().all(|c| c.is_ascii_alphabetic()) {
        return code.to_owned();
    }
    // Le suffixe doit rester une lettre de sous-jeu suivie de chiffres.
    let lettres = suffixe.chars().take_while(char::is_ascii_uppercase).count();
    let chiffres = suffixe.get(lettres..).unwrap_or("");
    if chiffres.is_empty() || !chiffres.chars().all(|c| c.is_ascii_digit()) {
        return code.to_owned();
    }
    format!("{prefixe}{}{suffixe}", langue.code())
}

/// Le libellé de langue d'un tirage **OCG**, d'après le suffixe de son code.
///
/// Scanflip n'a pas de libellé pour l'OCG : ceux-ci sont écrits dans son
/// style (« langue (région) », en français) pour qu'un tableur les lise
/// d'un coup d'œil. `None` pour un code TCG.
///
/// ```
/// use ygo_app::export::libelle_ocg;
/// assert_eq!(libelle_ocg("LOCR-JP001"), Some("Japonais (Japon)"));
/// assert_eq!(libelle_ocg("CROS-KR001"), Some("Coréen (Corée)"));
/// assert_eq!(libelle_ocg("RA02-EN001"), None);
/// ```
#[must_use]
pub fn libelle_ocg(code: &str) -> Option<&'static str> {
    if !a_suffixe_ocg(code) {
        return None;
    }
    let suffixe: String = code
        .rsplit_once('-')
        .map(|(_, s)| s)
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    Some(match suffixe.as_str() {
        "JP" => "Japonais (Japon)",
        "JA" => "Japonais (Asie)",
        "KR" | "KO" => "Coréen (Corée)",
        "AE" => "Anglais (Asie)",
        "SC" => "Chinois simplifié (Chine)",
        "TC" => "Chinois traditionnel (Asie)",
        _ => "OCG",
    })
}

/// Ce qu'un export contient, et ce qu'il a laissé de côté.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Le format écrit.
    pub format: Format,
    /// Classeurs que le format n'exporte pas — les classeurs OCG d'un export
    /// Scanflip.
    pub ecartes: Vec<String>,
    /// Lignes écrites.
    pub lignes: usize,
    /// Somme des quantités.
    pub exemplaires: i64,
    /// Lignes dont l'état n'est pas renseigné — elles sortent avec leurs
    /// trois colonnes d'édition vides.
    pub sans_etat: usize,
    /// Classeurs OCG présents dans le fichier — l'application les relit,
    /// Scanflip ne les connaît pas.
    pub ocg: Vec<String>,
    /// Lignes dont le code de rareté n'est pas dans les règles de Scanflip
    /// — elles seront **rejetées** par son import.
    pub raretes_refusees: Vec<String>,
}

/// Compte ce qu'un jeu de lignes contient.
#[must_use]
pub fn bilan(lignes: &[Sortie]) -> Bilan {
    let mut ocg: Vec<String> = lignes
        .iter()
        .filter(|l| a_suffixe_ocg(&l.code))
        .map(|l| l.extension.clone())
        .collect();
    ocg.sort_unstable();
    ocg.dedup();
    Bilan {
        format: if lignes.iter().any(|l| l.complement.is_some()) {
            Format::Application
        } else {
            Format::Scanflip
        },
        ecartes: Vec::new(),
        lignes: lignes.len(),
        exemplaires: lignes.iter().map(|l| l.quantite).sum(),
        sans_etat: lignes
            .iter()
            .filter(|l| l.qualite.trim().is_empty())
            .count(),
        ocg,
        // Une rareté hors des règles de Scanflip n'est pas une erreur de
        // notre part — `GMR` est une vraie rareté OCG. Mais la ligne sera
        // perdue en silence à l'arrivée, et mieux vaut le savoir avant
        // d'envoyer le fichier.
        raretes_refusees: {
            let mut v: Vec<String> = lignes
                .iter()
                .map(|l| l.rarete.clone())
                .filter(|r| !r.is_empty() && !rarity::scanflip::code_accepte(r))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        },
    }
}

/// Le rang d'artwork de chaque ligne de classeur : `(classeur, rowid)` → rang.
///
/// # Le calcul n'est pas refait ici
///
/// Il est **emprunté à l'import** : [`import::index`] est inversé, et rien
/// d'autre n'est calculé. Deux calculs jumeaux finissent toujours par
/// diverger d'un correctif, et il suffirait d'un rang d'écart pour qu'une
/// carte exportée en rang 1 se réimporte sur la ligne du rang 0.
///
/// # Pourquoi la base entière, et non les seules cartes possédées
///
/// Le rang se lisait auparavant sur la liste des **possédées**. Deux
/// artworks d'une même rareté dont l'utilisateur ne possède que le second :
/// l'export ne voyait qu'une ligne et l'écrivait en rang 0, quand l'import,
/// qui lit la base entière, attend le rang 1 pour cette ligne-là. La carte
/// revenait sur la ligne du premier artwork — silencieusement, et seulement
/// pour les collections incomplètes, c'est-à-dire toutes.
///
/// # Pourquoi une carte indexée par `(classeur, rowid)`
///
/// Un `rowid` n'est unique **que dans sa base**. Chaque classeur a la
/// sienne, et huit classeurs ont tous une ligne 1. Une carte indexée par le
/// seul `rowid` faisait que le rang du `RA02` numéro 1 écrasait celui du
/// `SDLI` numéro 1 : à l'export, les deux artworks d'une même rareté
/// sortaient tous deux en rang 0, et le réimport les fusionnait. Le défaut
/// n'est apparu qu'à l'aller-retour sur les vraies données.
#[must_use]
pub fn rangs(bases: &HashMap<String, Vec<import::LigneBase>>) -> HashMap<(String, i64), u32> {
    let mut rangs = HashMap::new();
    for (classeur, base) in bases {
        for ((_, rang, _), rowid) in import::index(base) {
            rangs.insert((classeur.clone(), rowid), rang);
        }
    }
    rangs
}

/// Lit la base complète des classeurs à exporter, pour le calcul des rangs.
///
/// # Errors
///
/// Rend une erreur au premier classeur illisible.
pub fn bases_de(
    paths: &Paths,
    classeurs: &[String],
) -> Result<HashMap<String, Vec<import::LigneBase>>> {
    let mut bases = HashMap::new();
    for c in classeurs {
        bases.insert(c.clone(), import::lignes_de_base(paths, c)?);
    }
    Ok(bases)
}

/// Transforme les lignes possédées en lignes Scanflip.
///
/// Une rareté que le référentiel ne connaît pas est **écrite telle quelle**
/// plutôt qu'omise : mieux vaut une ligne que l'utilisateur saura
/// interpréter qu'une carte disparue de son export.
#[must_use]
pub fn lignes(
    possedees: &[Possedee],
    langue: Langue,
    rangs: &HashMap<(String, i64), u32>,
) -> Vec<Sortie> {
    possedees
        .iter()
        .map(|p| {
            // Une ligne absente de la carte des rangs n'existe pas dans la
            // base qu'on a lue : plutôt que de la taire, on l'écrit en rang
            // 0 — l'export d'une carte vaut mieux que son escamotage.
            let rang = rangs
                .get(&(p.classeur.clone(), p.rowid))
                .copied()
                .unwrap_or(0);
            let nom = match langue {
                Langue::Francais if !p.name_fr.is_empty() => p.name_fr.clone(),
                _ => p.name.clone(),
            };
            Sortie {
                // Par ligne : une carte japonaise reste japonaise, quelle que
                // soit la langue du fichier (cf. tête de module).
                langue: libelle_ocg(&p.set_code)
                    .unwrap_or_else(|| langue.libelle())
                    .to_owned(),
                extension: p.classeur.clone(),
                code: traduire_code(&p.set_code, langue),
                nom,
                rarete: reference::nom_vers_code(&p.rarity)
                    .map_or_else(|| p.rarity.clone(), ToOwned::to_owned),
                edition: Edition::depuis_code(&p.edition),
                // `NM+` en base doit sortir en `NM` : les deux écritures
                // désignent le même état, et n'en garder qu'une évite
                // qu'un aller-retour fasse apparaître deux états là où il
                // n'y en a qu'un.
                qualite: rarity::scanflip::qualite_de(&p.qualite)
                    .map_or_else(|| p.qualite.clone(), ToOwned::to_owned),
                quantite: p.quantite.max(1),
                artwork: rang,
                complement: None,
            }
        })
        .collect()
}

/// Transforme les lignes possédées en lignes de l'export « Application ».
///
/// Les onze colonnes de Scanflip, à une différence près : le **code reste
/// celui de la base** (`RA04-EN002`, `CORI-JPS10`), jamais traduit — c'est
/// lui que l'autre installation connaît. Le complément dit le reste : la
/// rareté exacte, le nom anglais, l'illustration, son identifiant et le
/// cadre.
#[must_use]
pub fn lignes_application(
    possedees: &[Possedee],
    langue: Langue,
    rangs: &HashMap<(String, i64), u32>,
) -> Vec<Sortie> {
    lignes(possedees, langue, rangs)
        .into_iter()
        .zip(possedees)
        .map(|(sortie, p)| Sortie {
            code: p.set_code.clone(),
            complement: Some(Complement {
                rarete: p.rarity.clone(),
                nom_anglais: p.name.clone(),
                illustration: import::illustration_de(&p.image),
                id_image: p.card_image_id,
                overframe: p.overframe,
            }),
            ..sortie
        })
        .collect()
}

/// Les classeurs qu'un format exporte, et ceux qu'il écarte.
///
/// Scanflip ne connaît pas l'OCG : un export Scanflip laisse les classeurs
/// OCG de côté — et leurs tirages anglais avec eux (`CORI-ENS10`, rangé dans
/// `CORI-JP`), que Scanflip recevrait sous un code qu'il ne relie à rien.
///
/// ```
/// use ygo_app::export::{classeurs_du_format, Format};
/// let tous = vec!["CORI-JP".to_owned(), "RA04".to_owned()];
/// assert_eq!(
///     classeurs_du_format(Format::Scanflip, &tous),
///     (vec!["RA04".to_owned()], vec!["CORI-JP".to_owned()])
/// );
/// assert_eq!(classeurs_du_format(Format::Application, &tous), (tous.clone(), vec![]));
/// ```
#[must_use]
pub fn classeurs_du_format(format: Format, classeurs: &[String]) -> (Vec<String>, Vec<String>) {
    match format {
        Format::Application => (classeurs.to_vec(), Vec::new()),
        Format::Scanflip => classeurs.iter().cloned().partition(|c| !a_suffixe_ocg(c)),
    }
}

/// Un export prêt à écrire : ses lignes, et ce qu'il en faut savoir.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Export {
    /// Les lignes, dans l'ordre des classeurs puis du classeur.
    pub lignes: Vec<Sortie>,
    /// Le bilan, classeurs écartés compris.
    pub bilan: Bilan,
}

/// Prépare l'export de `classeurs` au `format` donné.
///
/// Les rangs se lisent sur la base entière des classeurs exportés, pas sur
/// les seules cartes possédées : c'est la même numérotation que l'import.
///
/// # Errors
///
/// Rend une erreur au premier classeur illisible.
pub fn preparer(
    paths: &Paths,
    classeurs: &[String],
    format: Format,
    langue: Langue,
) -> Result<Export> {
    let (retenus, ecartes) = classeurs_du_format(format, classeurs);
    let rangs = rangs(&bases_de(paths, &retenus)?);
    let possedees = possedees_de(paths, &retenus)?;
    let lignes = match format {
        Format::Scanflip => lignes(&possedees, langue, &rangs),
        Format::Application => lignes_application(&possedees, langue, &rangs),
    };
    let mut bilan = bilan(&lignes);
    bilan.format = format;
    bilan.ecartes = ecartes;
    if format == Format::Application {
        // Les règles de Scanflip ne concernent pas un fichier qui ne lui est
        // pas destiné : l'application relit toutes ses raretés.
        bilan.raretes_refusees.clear();
    }
    Ok(Export { lignes, bilan })
}

/// Lit les cartes possédées d'un classeur.
///
/// Pas de filtre sur l'image : une carte possédée dont l'illustration
/// manque reste une carte de la collection.
///
/// # Une ligne par état — 2026-10-01
///
/// Une ligne de classeur dont des exemplaires sont réglés à part
/// ([`crate::exemplaires`]) rend **une `Possedee` par état** : cinq
/// exemplaires dont deux `PL` sortent en deux lignes Scanflip, `NM ×3` et
/// `PL ×2`, sur le même rang d'artwork. C'est ce que l'import sait relire
/// sans rien perdre ([`import::Ecriture::a_part`]).
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn possedees(paths: &Paths, classeur: &str) -> Result<Vec<Possedee>> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(classeur))
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let mut requete = conn
        .prepare(
            "SELECT rowid, set_code, rarity, card_image_id, name, name_fr,
                    quantite, qualite, edition, card_image_url, extended_art
               FROM cards
              WHERE possessed = 1
              ORDER BY sort_order, rowid",
        )
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let mut a_part = crate::exemplaires::a_part_du_classeur(&conn)?;
    let lignes: Vec<Possedee> = requete
        .query_map([], |l| {
            Ok(Possedee {
                rowid: l.get(0)?,
                classeur: classeur.to_owned(),
                set_code: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                rarity: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                card_image_id: l.get(3)?,
                name: l.get::<_, Option<String>>(4)?.unwrap_or_default(),
                name_fr: l.get::<_, Option<String>>(5)?.unwrap_or_default(),
                quantite: l.get::<_, Option<i64>>(6)?.unwrap_or(1),
                qualite: l.get::<_, Option<String>>(7)?.unwrap_or_default(),
                edition: l.get::<_, Option<String>>(8)?.unwrap_or_default(),
                image: l.get::<_, Option<String>>(9)?.unwrap_or_default(),
                overframe: l.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    Ok(lignes
        .into_iter()
        .flat_map(|p| {
            let reglees = a_part.remove(&p.rowid).unwrap_or_default();
            par_etat(p, reglees)
        })
        .collect())
}

/// Une ligne possédée, éclatée en une `Possedee` par état de ses
/// exemplaires. Sans exemplaire réglé à part, elle se rend telle quelle.
fn par_etat(p: Possedee, a_part: Vec<(i64, crate::exemplaires::Etat)>) -> Vec<Possedee> {
    if a_part.is_empty() {
        return vec![p];
    }
    let exemplaires = crate::exemplaires::Exemplaires::depuis(
        p.quantite.max(1),
        crate::exemplaires::Etat::nouveau(&p.qualite, &p.edition),
        a_part,
    );
    exemplaires
        .groupes()
        .into_iter()
        .map(|(etat, n)| Possedee {
            quantite: n,
            qualite: etat.qualite,
            edition: etat.edition,
            ..p.clone()
        })
        .collect()
}

/// Lit les cartes possédées de plusieurs classeurs, dans l'ordre donné.
///
/// # Errors
///
/// Rend une erreur au premier classeur illisible.
pub fn possedees_de(paths: &Paths, classeurs: &[String]) -> Result<Vec<Possedee>> {
    let mut tout = Vec::new();
    for c in classeurs {
        tout.extend(possedees(paths, c)?);
    }
    Ok(tout)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::scanflip;

    fn possedee(rowid: i64, set_code: &str, rarity: &str, img: i64) -> Possedee {
        Possedee {
            rowid,
            classeur: set_code.split('-').next().unwrap_or("").to_owned(),
            set_code: set_code.to_owned(),
            rarity: rarity.to_owned(),
            card_image_id: Some(img),
            name: "Rescue Cat".into(),
            name_fr: "Chat Sauveteur".into(),
            quantite: 1,
            qualite: "NM".into(),
            edition: "1st".into(),
            ..Possedee::default()
        }
    }

    /// La base d'un classeur, telle que l'import la lit.
    fn base_de(rows: &[(i64, &str, &str, i64)]) -> HashMap<String, Vec<import::LigneBase>> {
        let mut bases: HashMap<String, Vec<import::LigneBase>> = HashMap::new();
        for (rowid, set_code, rarity, img) in rows {
            let classeur = set_code.split('-').next().unwrap_or("").to_owned();
            bases.entry(classeur).or_default().push(import::LigneBase {
                rowid: *rowid,
                set_code: (*set_code).to_owned(),
                rarity: (*rarity).to_owned(),
                card_image_id: Some(*img),
                name: "Rescue Cat".into(),
                name_fr: Some("Chat Sauveteur".into()),
                ..import::LigneBase::default()
            });
        }
        bases
    }

    /// Les lignes Scanflip d'une seule carte, dont le classeur ne contient
    /// qu'elle — le raccourci de la plupart des tests de format.
    fn ligne_seule(p: &Possedee, langue: Langue) -> Sortie {
        let rangs = rangs(&base_des(std::slice::from_ref(p)));
        lignes(std::slice::from_ref(p), langue, &rangs)
            .into_iter()
            .next()
            .unwrap()
    }

    /// Le cas courant des tests : tout ce que l'utilisateur possède est
    /// tout ce que le classeur contient.
    fn base_des(possedees: &[Possedee]) -> HashMap<String, Vec<import::LigneBase>> {
        let rows: Vec<(i64, &str, &str, i64)> = possedees
            .iter()
            .map(|p| {
                (
                    p.rowid,
                    p.set_code.as_str(),
                    p.rarity.as_str(),
                    p.card_image_id.unwrap_or(0),
                )
            })
            .collect();
        base_de(&rows)
    }

    /// Les libellés sont ceux de Scanflip, pas ceux du Python.
    #[test]
    fn le_libelle_de_langue_est_celui_de_scanflip() {
        assert_eq!(Langue::Francais.libelle(), "Français (France)");
        assert_eq!(
            Langue::Anglais.libelle(),
            "Anglais (Monde)",
            "et non « English », que le Python écrivait"
        );
    }

    /// Les codes se traduisent, sauf les OCG.
    #[test]
    fn les_codes_ocg_ne_se_traduisent_pas() {
        assert_eq!(traduire_code("RA02-EN001", Langue::Francais), "RA02-FR001");
        assert_eq!(traduire_code("LDK2-ENJ01", Langue::Francais), "LDK2-FRJ01");
        assert_eq!(traduire_code("SDLI-FR001", Langue::Anglais), "SDLI-EN001");
        for ocg in ["LOCH-JP001", "LOCR-JP012", "CROS-KR001"] {
            assert_eq!(
                traduire_code(ocg, Langue::Francais),
                ocg,
                "{ocg} n'existe pas en français"
            );
        }
        assert_eq!(traduire_code("", Langue::Francais), "");
        assert_eq!(traduire_code("PROMO", Langue::Francais), "PROMO");
    }

    /// Le nom français est préféré quand il existe, et seulement alors.
    #[test]
    fn le_nom_suit_la_langue_avec_repli() {
        let mut p = possedee(1, "RA02-EN001", "Secret Rare", 1);
        assert_eq!(ligne_seule(&p, Langue::Francais).nom, "Chat Sauveteur");
        assert_eq!(ligne_seule(&p, Langue::Anglais).nom, "Rescue Cat");
        p.name_fr = String::new();
        assert_eq!(
            ligne_seule(&p, Langue::Francais).nom,
            "Rescue Cat",
            "sans nom français, l'anglais tient lieu"
        );
    }

    /// Une rareté hors référentiel est écrite telle quelle plutôt
    /// qu'escamotée.
    #[test]
    fn une_rarete_inconnue_traverse_sans_disparaitre() {
        let p = possedee(1, "RA02-EN001", "Rareté Martienne", 1);
        let s = [ligne_seule(&p, Langue::Francais)];
        assert_eq!(s.len(), 1, "la carte reste dans l'export");
        assert_eq!(s[0].rarete, "Rareté Martienne");
    }

    /// L'export et l'import numérotent les artworks **de la même façon**.
    ///
    /// C'est la propriété qui fait tenir l'aller-retour : si les deux
    /// calculs divergeaient, une carte exportée en rang 1 se réimporterait
    /// sur la ligne du rang 0.
    #[test]
    fn l_export_et_l_import_numerotent_pareil() {
        let base = [
            possedee(1, "RA02-EN001", "Collector's Rare", -1_276_398_914),
            possedee(2, "RA02-EN001", "Super Rare", -1_074_569_406),
            possedee(3, "RA02-EN001", "Collector's Rare", 14_878_871),
            possedee(4, "RA02-EN001", "Super Rare", 14_878_871),
        ];
        let bases = base_des(&base);
        let rangs_export = rangs(&bases);
        let index_import = import::index(&bases["RA02"]);
        for p in &base {
            let rang = rangs_export[&(p.classeur.clone(), p.rowid)];
            let vise = index_import
                .get(&(p.set_code.clone(), rang, p.rarity.clone()))
                .copied();
            assert_eq!(
                vise,
                Some(p.rowid),
                "{} {} rang {rang} doit revenir sur la ligne {}",
                p.set_code,
                p.rarity,
                p.rowid
            );
        }
    }

    /// Le rang se lit sur la **base entière**, pas sur les seules cartes
    /// possédées.
    ///
    /// # Le défaut que ce test fige
    ///
    /// Le rang se calculait sur la liste des possédées. Deux artworks d'une
    /// même rareté dont l'utilisateur ne possède que le **second** : l'export
    /// ne voyait qu'une ligne et l'écrivait en rang 0, quand l'import, qui
    /// lit la base entière, attend le rang 1 pour cette ligne-là. La carte
    /// revenait sur la ligne du premier artwork — celui que l'utilisateur ne
    /// possède pas. Silencieux, et vrai de toute collection incomplète.
    #[test]
    fn le_rang_se_lit_sur_la_base_entiere_et_non_sur_les_possedees() {
        // Le classeur a les deux artworks ; l'utilisateur n'a que le second.
        let bases = base_de(&[
            (1, "RA02-EN001", "Ultra Rare", -1_276_398_914),
            (2, "RA02-EN001", "Ultra Rare", 14_878_871),
        ]);
        let possedees = vec![possedee(2, "RA02-EN001", "Ultra Rare", 14_878_871)];

        let sorties = lignes(&possedees, Langue::Francais, &rangs(&bases));
        assert_eq!(sorties[0].artwork, 1, "le second artwork sort en rang 1");

        // Et il revient bien sur SA ligne au réimport.
        let octets = scanflip::ecrire_octets(&sorties).unwrap();
        let relu = scanflip::lire_octets(&octets).unwrap();
        let rapport = import::planifier(&relu.lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert_eq!(
            rapport.ecritures[0].rowid, 2,
            "et non la ligne 1, que l'utilisateur ne possède pas"
        );
    }

    /// Trois exemplaires d'une même carte, une seule image : chacun a son
    /// rang.
    ///
    /// # Le défaut que ce test fige
    ///
    /// `LDK2-ENK01` porte trois lignes de classeur — Kaiba a trois Dragons
    /// Blancs aux Yeux Bleus — mais les sources leur donnent un seul
    /// `card_image_id`. En numérotant les **images**, les trois lignes
    /// partageaient le rang 0 : l'import refusait les exemplaires 2 et 3
    /// (`ArtworkAbsent`) et l'export les écrivait tous en rang 0. Un
    /// aller-retour perdait deux cartes sur trois.
    #[test]
    fn trois_exemplaires_d_une_meme_image_ont_chacun_leur_rang() {
        let bases = base_de(&[
            (44, "LDK2-ENK01", "Common", 89_631_139),
            (131, "LDK2-ENK01", "Common", 89_631_139),
            (132, "LDK2-ENK01", "Common", 89_631_139),
        ]);
        let possedees: Vec<Possedee> = [44, 131, 132]
            .into_iter()
            .map(|rowid| possedee(rowid, "LDK2-ENK01", "Common", 89_631_139))
            .collect();

        let sorties = lignes(&possedees, Langue::Francais, &rangs(&bases));
        assert_eq!(
            sorties.iter().map(|s| s.artwork).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "trois lignes, trois rangs"
        );

        // Et l'aller-retour rend les trois lignes, pas une seule.
        let octets = scanflip::ecrire_octets(&sorties).unwrap();
        let relu = scanflip::lire_octets(&octets).unwrap();
        let rapport = import::planifier(&relu.lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert!(
            rapport.fusions.is_empty(),
            "aucune ne se pose sur une autre"
        );
        let mut retrouves: Vec<i64> = rapport.ecritures.iter().map(|e| e.rowid).collect();
        retrouves.sort_unstable();
        assert_eq!(retrouves, vec![44, 131, 132]);
        for e in &rapport.ecritures {
            assert_eq!(e.quantite, 1, "un exemplaire chacune, pas trois sur une");
        }
    }

    /// Deux classeurs numérotent leurs lignes à partir de 1 chacun.
    ///
    /// # Le défaut que ce test fige
    ///
    /// Les rangs étaient rendus dans une carte indexée par le seul `rowid`.
    /// Comme chaque base a sa propre numérotation, la ligne 1 de `SDLI`
    /// écrasait celle de `RA02` : les deux artworks d'une même rareté
    /// sortaient tous deux en rang 0, et le réimport les fusionnait sur une
    /// seule ligne — deux exemplaires perdus par carte. Seul un
    /// aller-retour sur plusieurs classeurs réels l'a montré ; tous les
    /// tests d'alors ne portaient que sur un classeur.
    #[test]
    fn deux_classeurs_aux_rowid_identiques_ne_se_marchent_pas_dessus() {
        let base = vec![
            possedee(1, "RA02-EN001", "Secret Rare", -1_276_398_914),
            possedee(2, "RA02-EN001", "Secret Rare", 14_878_871),
            // SDLI a lui aussi une ligne 1 et une ligne 2.
            possedee(1, "SDLI-EN001", "Common", 500),
            possedee(2, "SDLI-EN002", "Common", 600),
        ];
        let rangs = rangs(&base_des(&base));
        assert_eq!(
            base.iter()
                .map(|p| rangs[&(p.classeur.clone(), p.rowid)])
                .collect::<Vec<_>>(),
            vec![0, 1, 0, 0],
            "les deux artworks de RA02 gardent leurs rangs 0 et 1"
        );
        let sorties = lignes(&base, Langue::Francais, &rangs);
        assert_eq!(sorties[0].artwork, 0);
        assert_eq!(sorties[1].artwork, 1, "le second artwork n'est pas écrasé");
    }

    /// L'aller-retour complet : exporter, écrire, relire, réapparier —
    /// chaque carte retrouve sa ligne.
    #[test]
    fn un_aller_retour_complet_remet_chaque_carte_a_sa_place() {
        let base = vec![
            possedee(1, "RA02-EN001", "Collector's Rare", -1_276_398_914),
            possedee(2, "RA02-EN001", "Super Rare", -1_074_569_406),
            possedee(3, "RA02-EN001", "Collector's Rare", 14_878_871),
            possedee(4, "RA02-EN001", "Super Rare", 14_878_871),
            possedee(5, "RA02-EN005", "Ultimate Rare", 42),
        ];
        let bases = base_des(&base);
        let octets =
            scanflip::ecrire_octets(&lignes(&base, Langue::Francais, &rangs(&bases))).unwrap();
        let relu = scanflip::lire_octets(&octets).unwrap();
        assert!(relu.illisibles.is_empty());
        assert_eq!(relu.lignes.len(), base.len());

        let rapport = import::planifier(&relu.lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert!(rapport.fusions.is_empty(), "aucune collision");
        let mut retrouves: Vec<i64> = rapport.ecritures.iter().map(|e| e.rowid).collect();
        retrouves.sort_unstable();
        assert_eq!(retrouves, vec![1, 2, 3, 4, 5]);
        // Et les quantités et états sont revenus intacts.
        for e in &rapport.ecritures {
            assert_eq!(e.quantite, 1);
            assert_eq!(e.qualite, "NM");
            assert_eq!(e.edition, Some(Edition::Premiere));
        }
    }

    /// Cinq exemplaires dont deux `PL Unlimited` : deux lignes Scanflip à
    /// l'export, et l'import les rend tels quels — quantité, état commun et
    /// exemplaires à part.
    #[test]
    fn des_exemplaires_regles_a_part_font_l_aller_retour() {
        use crate::exemplaires::Etat;
        let mut p = possedee(1, "RA02-EN001", "Super Rare", 42);
        p.quantite = 5;
        let pl = Etat::nouveau("PL", "unlimited");
        let eclatees = par_etat(p.clone(), vec![(7, pl.clone()), (8, pl.clone())]);
        assert_eq!(eclatees.len(), 2, "une ligne par état");
        assert_eq!(
            eclatees
                .iter()
                .map(|e| (e.qualite.as_str(), e.quantite))
                .collect::<Vec<_>>(),
            vec![("NM", 3), ("PL", 2)]
        );
        assert!(
            par_etat(p.clone(), Vec::new()) == vec![p.clone()],
            "sans réglage, inchangée"
        );

        let bases = base_des(std::slice::from_ref(&p));
        let octets =
            scanflip::ecrire_octets(&lignes(&eclatees, Langue::Francais, &rangs(&bases))).unwrap();
        let relu = scanflip::lire_octets(&octets).unwrap();
        let rapport = import::planifier(&relu.lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert_eq!(rapport.ecritures.len(), 1, "une seule ligne de classeur");
        let e = &rapport.ecritures[0];
        assert_eq!(e.quantite, 5);
        assert_eq!(e.commun(), Etat::nouveau("NM", "1st"));
        assert_eq!(e.a_part, vec![(pl, 2)]);
        assert_eq!(rapport.fusions_avec_perte().count(), 0, "rien ne se perd");
    }

    /// Une carte OCG sort avec son code natif et le libellé de **sa**
    /// langue, à côté des cartes TCG du fichier ; le bilan nomme son
    /// classeur.
    #[test]
    fn les_classeurs_ocg_sont_exportes_avec_leur_langue() {
        let mut jp = possedee(1, "LOCR-JP001", "Ultra Rare", 1);
        jp.classeur = "LOCR-JP".into();
        let tcg = possedee(2, "RA05-EN001", "Ultra Rare", 2);
        let mut base = base_des(std::slice::from_ref(&tcg));
        base.insert(
            "LOCR-JP".into(),
            vec![import::LigneBase {
                rowid: 1,
                set_code: "LOCR-JP001".into(),
                rarity: "Ultra Rare".into(),
                card_image_id: Some(1),
                name: "Rescue Cat".into(),
                name_fr: None,
                ..import::LigneBase::default()
            }],
        );
        let sortie = lignes(&[jp, tcg], Langue::Francais, &rangs(&base));
        assert_eq!(sortie[0].code, "LOCR-JP001", "code natif");
        assert_eq!(sortie[0].langue, "Japonais (Japon)");
        assert_eq!(sortie[0].extension, "LOCR-JP");
        assert_eq!(sortie[1].code, "RA05-FR001");
        assert_eq!(sortie[1].langue, "Français (France)");
        let b = bilan(&sortie);
        assert_eq!(b.ocg, vec!["LOCR-JP".to_owned()]);
        // Un classeur dont le nom contient « JP » sans en être le suffixe
        // n'est pas OCG.
        assert_eq!(libelle_ocg("JPEG-EN001"), None);
    }

    /// Exportée puis relue par l'import, une carte OCG retrouve sa ligne.
    #[test]
    fn une_carte_ocg_fait_l_aller_retour() {
        let mut jp = possedee(7, "LOCH-JP013", "Grand Master Rare", 9);
        jp.classeur = "LOCH-JP".into();
        let base = HashMap::from([(
            "LOCH-JP".to_owned(),
            vec![import::LigneBase {
                rowid: 7,
                set_code: "LOCH-JP013".into(),
                rarity: "Grand Master Rare".into(),
                card_image_id: Some(9),
                name: "Rescue Cat".into(),
                name_fr: None,
                ..import::LigneBase::default()
            }],
        )]);
        let sortie = lignes(&[jp], Langue::Francais, &rangs(&base));
        let octets = scanflip::ecrire_octets(&sortie).unwrap();
        let lecture = scanflip::lire_octets(&octets).unwrap();
        let rapport = import::planifier(&lecture.lignes, &base);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert_eq!(rapport.ecritures.len(), 1);
        assert_eq!(rapport.ecritures[0].rowid, 7);
    }

    /// Le bilan compte les lignes sans état — c'est ce que l'export doit
    /// annoncer avant d'écrire.
    #[test]
    fn le_bilan_compte_les_lignes_sans_etat() {
        let mut avec = possedee(1, "RA02-EN001", "Secret Rare", 1);
        avec.quantite = 2;
        let mut sans = possedee(2, "RA02-EN002", "Secret Rare", 2);
        sans.qualite = String::new();
        let mut espaces = possedee(3, "RA02-EN003", "Secret Rare", 3);
        espaces.qualite = "   ".into();

        let ensemble = [avec, sans, espaces];
        let lignes = lignes(&ensemble, Langue::Francais, &rangs(&base_des(&ensemble)));
        let b = bilan(&lignes);
        assert_eq!(b.lignes, 3);
        assert_eq!(b.exemplaires, 4, "2 + 1 + 1");
        assert_eq!(b.sans_etat, 2, "la vide et celle qui n'a que des espaces");
        assert!(b.ocg.is_empty());
    }

    /// Une ligne sans état sort avec ses **trois** colonnes d'édition
    /// vides — l'export n'invente pas un « NM » que personne n'a constaté.
    #[test]
    fn une_ligne_sans_etat_ne_se_voit_pas_inventer_un_etat() {
        let mut p = possedee(1, "RA05-EN001", "Ultra Rare", 1);
        p.qualite = String::new();
        p.edition = String::new();
        let cellules = ligne_seule(&p, Langue::Francais).cellules();
        assert!(
            cellules[5..8].iter().all(String::is_empty),
            "aucune colonne d'édition n'est remplie : {:?}",
            &cellules[5..8]
        );
        assert_eq!(cellules[8], "1", "la quantité, elle, est bien là");
    }

    /// Une rareté que Scanflip refuse est signalée avant l'envoi.
    ///
    /// `GMR` est une vraie rareté OCG, présente dans la table du Python et
    /// absente des règles de Scanflip. Notre export l'écrit — c'est ce que
    /// la base contient — mais il dit que la ligne sera rejetée.
    #[test]
    fn une_rarete_hors_des_regles_de_scanflip_est_signalee() {
        let ocg = possedee(1, "RA02-EN001", "Grand Master Rare", 1);
        let tcg = possedee(2, "RA02-EN002", "Secret Rare", 2);
        let ensemble = [ocg, tcg];
        let lignes = lignes(&ensemble, Langue::Francais, &rangs(&base_des(&ensemble)));
        assert_eq!(lignes[0].rarete, "GMR", "elle sort telle quelle");
        let b = bilan(&lignes);
        assert_eq!(b.raretes_refusees, vec!["GMR".to_owned()]);
        assert_eq!(b.lignes, 2, "la ligne n'est pas retirée pour autant");
    }

    /// `NM+` sort en `NM` : une seule écriture par état.
    #[test]
    fn les_etats_sortent_dans_leur_ecriture_courte() {
        let mut p = possedee(1, "RA02-EN001", "Secret Rare", 1);
        p.qualite = "NM+".into();
        assert_eq!(ligne_seule(&p, Langue::Francais).qualite, "NM");
        p.qualite = "Near Mint".into();
        assert_eq!(ligne_seule(&p, Langue::Francais).qualite, "NM");
        // Un état inconnu du référentiel traverse sans être escamoté.
        p.qualite = "cabossée".into();
        assert_eq!(ligne_seule(&p, Langue::Francais).qualite, "cabossée");
    }

    /// L'aller-retour sur plusieurs classeurs est un **point fixe**.
    ///
    /// Exporter, réimporter, réexporter : le second fichier est identique
    /// au premier, et chaque ligne retrouve exactement la ligne de base
    /// dont elle vient. C'est la propriété que l'utilisateur attend en
    /// pratique — exporter vers Scanflip, y retoucher, réimporter — et
    /// c'est elle qui a révélé la collision de `rowid` entre classeurs.
    #[test]
    fn l_aller_retour_multi_classeurs_est_un_point_fixe() {
        let base = vec![
            possedee(1, "RA02-EN001", "Secret Rare", -1_276_398_914),
            possedee(2, "RA02-EN001", "Secret Rare", 14_878_871),
            possedee(3, "RA02-EN001", "Ultra Rare", -939_225_438),
            possedee(1, "SDLI-EN001", "Common", 500),
            possedee(2, "SDLI-EN002", "Common", 600),
        ];
        let premier =
            scanflip::ecrire_octets(&lignes(&base, Langue::Francais, &rangs(&base_des(&base))))
                .unwrap();
        let relu = scanflip::lire_octets(&premier).unwrap();
        assert_eq!(relu.lignes.len(), base.len());

        let mut bases: std::collections::HashMap<String, Vec<import::LigneBase>> =
            std::collections::HashMap::new();
        for p in &base {
            bases
                .entry(p.classeur.clone())
                .or_default()
                .push(import::LigneBase {
                    rowid: p.rowid,
                    set_code: p.set_code.clone(),
                    rarity: p.rarity.clone(),
                    card_image_id: p.card_image_id,
                    name: p.name.clone(),
                    name_fr: (!p.name_fr.is_empty()).then(|| p.name_fr.clone()),
                    ..import::LigneBase::default()
                });
        }
        let rapport = import::planifier(&relu.lignes, &bases);
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert!(
            rapport.fusions.is_empty(),
            "aucune ligne n'en écrase une autre : {:?}",
            rapport.fusions
        );
        assert_eq!(
            rapport.ecritures.len(),
            base.len(),
            "autant de lignes de base écrites que d'exemplaires exportés"
        );
        // Et le second export est mot pour mot le premier.
        let second =
            scanflip::ecrire_octets(&lignes(&base, Langue::Francais, &rangs(&base_des(&base))))
                .unwrap();
        assert_eq!(premier, second);
    }

    /// Une quantité nulle en base ne produit pas une ligne à zéro :
    /// Scanflip n'écrit que des exemplaires réels.
    #[test]
    fn une_quantite_absente_vaut_un_exemplaire() {
        let mut p = possedee(1, "RA02-EN001", "Secret Rare", 1);
        p.quantite = 0;
        assert_eq!(ligne_seule(&p, Langue::Francais).quantite, 1);
    }

    /// Une édition que la base n'a jamais renseignée s'écrit dans
    /// « 1st Edition », et se relit telle quelle.
    #[test]
    fn une_edition_vide_se_replie_sans_casser_l_aller_retour() {
        let mut p = possedee(1, "RA02-EN001", "Secret Rare", 1);
        p.edition = String::new();
        let sorties = [ligne_seule(&p, Langue::Francais)];
        assert_eq!(sorties[0].edition, None);
        let octets = scanflip::ecrire_octets(&sorties).unwrap();
        let relu = scanflip::lire_octets(&octets).unwrap();
        assert_eq!(relu.lignes[0].edition, Some(Edition::Premiere));
        assert_eq!(relu.lignes[0].qualite, "NM");
    }

    /// Une ligne de l'export « Application » : le code de la base, jamais
    /// traduit, et le complément qui désigne la ligne.
    #[test]
    fn l_export_application_garde_le_code_de_la_base_et_l_illustration() {
        let mut p = possedee(
            984,
            "RA04-EN002",
            "Quarter Century Secret Rare",
            -610_954_957,
        );
        p.image =
            "https://ms.yugipedia.com//b/bd/BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png".into();
        p.overframe = true;
        let rangs = rangs(&base_des(std::slice::from_ref(&p)));
        let scanflip = lignes(std::slice::from_ref(&p), Langue::Francais, &rangs);
        let application = lignes_application(std::slice::from_ref(&p), Langue::Francais, &rangs);
        assert_eq!(scanflip[0].code, "RA04-FR002");
        assert_eq!(scanflip[0].complement, None);
        let a = &application[0];
        assert_eq!(a.code, "RA04-EN002", "le code de la base");
        assert_eq!(a.rarete, scanflip[0].rarete, "la case Rareté ne change pas");
        assert_eq!(
            (a.langue.as_str(), a.nom.as_str(), a.quantite),
            (scanflip[0].langue.as_str(), scanflip[0].nom.as_str(), 1)
        );
        assert_eq!(
            a.complement,
            Some(Complement {
                rarete: "Quarter Century Secret Rare".into(),
                nom_anglais: "Rescue Cat".into(),
                illustration: "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png".into(),
                id_image: Some(-610_954_957),
                overframe: true,
            })
        );
        assert_eq!(bilan(&application).format, Format::Application);
        assert_eq!(bilan(&scanflip).format, Format::Scanflip);
    }

    /// Le cas qui a motivé le format : deux installations qui ne rangent pas
    /// les artworks d'un tirage dans le même ordre. Par le rang (Scanflip),
    /// les deux cartes échangent leurs lignes ; par l'illustration
    /// (Application), chacune retrouve la sienne.
    #[test]
    fn l_export_application_survit_a_un_autre_ordre_des_artworks() {
        // La source range le scan de base (976) avant l'AA (587) ; la cible a
        // les mêmes scans sous d'autres identifiants, et l'ordre inverse.
        let fichiers = [
            (
                976,
                -1_589_717_855,
                10,
                100,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
            ),
            (
                587,
                -610_954_957,
                11,
                99,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
            ),
        ];
        let rarete = "Quarter Century Secret Rare";
        let source: Vec<Possedee> = fichiers
            .iter()
            .map(|(rowid, img, _, _, fichier)| Possedee {
                image: format!("https://ms.yugipedia.com//a/ab/{fichier}"),
                ..possedee(*rowid, "RA04-EN108", rarete, *img)
            })
            .collect();
        let cible = HashMap::from([(
            "RA04".to_owned(),
            fichiers
                .iter()
                .map(|(_, _, rowid, img, fichier)| import::LigneBase {
                    rowid: *rowid,
                    set_code: "RA04-EN108".into(),
                    rarity: rarete.into(),
                    card_image_id: Some(*img),
                    name: "Blue-Eyes White Dragon".into(),
                    image: format!("https://ms.yugipedia.com//c/cd/{fichier}"),
                    ..import::LigneBase::default()
                })
                .collect::<Vec<_>>(),
        )]);
        let rangs_source = rangs(&base_des(&source));
        // La ligne de la cible où une ligne exportée retombe.
        let cible_de = |sortie: &Sortie| -> i64 {
            let octets = scanflip::ecrire_octets(std::slice::from_ref(sortie)).unwrap();
            let lecture = scanflip::lire_octets(&octets).unwrap();
            let r = import::planifier(&lecture.lignes, &cible);
            assert!(r.refusees.is_empty(), "{:?}", r.refusees);
            r.ecritures[0].rowid
        };
        let par_rang: Vec<i64> = lignes(&source, Langue::Francais, &rangs_source)
            .iter()
            .map(cible_de)
            .collect();
        let par_illustration: Vec<i64> =
            lignes_application(&source, Langue::Francais, &rangs_source)
                .iter()
                .map(cible_de)
                .collect();
        assert_eq!(
            par_rang,
            vec![11, 10],
            "par le rang, elles échangent leurs lignes"
        );
        assert_eq!(
            par_illustration,
            vec![10, 11],
            "par l'illustration, chacune la sienne"
        );
    }

    /// Une installation jetable : un classeur TCG et un classeur OCG.
    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        for (classeur, lignes) in [
            (
                "RA04",
                vec![(
                    984,
                    "RA04-EN002",
                    "Quarter Century Secret Rare",
                    "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
                )],
            ),
            (
                "CORI-JP",
                vec![
                    (
                        125,
                        "CORI-JPS10",
                        "Ultra Rare",
                        "MelffyMommy-CORI-JP-UR.png",
                    ),
                    (170, "CORI-JP080", "Normal Rare", ""),
                    (171, "CORI-ENS10", "Grand Master Rare", ""),
                ],
            ),
        ] {
            std::fs::create_dir_all(paths.dossier_classeur(classeur)).unwrap();
            let conn = rusqlite::Connection::open(paths.classeur_db(classeur)).unwrap();
            conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            for (rowid, set_code, rarity, fichier) in lignes {
                conn.execute(
                    "INSERT INTO cards (rowid, name, set_code, rarity, card_image_id, card_image_url,
                                        possessed, quantite, qualite, edition)
                     VALUES (?1, 'Carte', ?2, ?3, ?1, ?4, 1, 1, 'NM', '1st')",
                    rusqlite::params![
                        rowid,
                        set_code,
                        rarity,
                        if fichier.is_empty() {
                            String::new()
                        } else {
                            format!("https://ms.yugipedia.com//a/ab/{fichier}")
                        }
                    ],
                )
                .unwrap();
            }
        }
        (tmp, paths)
    }

    /// Scanflip écarte les classeurs OCG et le dit ; Application les garde,
    /// codes de la base compris — et se relit sans un refus.
    #[test]
    fn chaque_format_choisit_ses_classeurs() {
        let (_tmp, paths) = installation();
        let classeurs = vec!["CORI-JP".to_owned(), "RA04".to_owned()];

        let scanflip = preparer(&paths, &classeurs, Format::Scanflip, Langue::Francais).unwrap();
        assert_eq!(scanflip.bilan.format, Format::Scanflip);
        assert_eq!(scanflip.bilan.ecartes, vec!["CORI-JP".to_owned()]);
        assert_eq!(scanflip.lignes.len(), 1);
        assert_eq!(scanflip.lignes[0].code, "RA04-FR002");
        assert!(scanflip.bilan.ocg.is_empty());

        let application =
            preparer(&paths, &classeurs, Format::Application, Langue::Francais).unwrap();
        assert_eq!(application.bilan.format, Format::Application);
        assert!(application.bilan.ecartes.is_empty());
        assert_eq!(application.bilan.ocg, vec!["CORI-JP".to_owned()]);
        let codes: Vec<&str> = application.lignes.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(
            codes,
            vec!["CORI-JPS10", "CORI-JP080", "CORI-ENS10", "RA04-EN002"]
        );
        assert!(
            application.bilan.raretes_refusees.is_empty(),
            "les règles de Scanflip ne concernent pas ce fichier"
        );
        assert_eq!(
            application.lignes[0]
                .complement
                .as_ref()
                .map(|c| c.illustration.as_str()),
            Some("MelffyMommy-CORI-JP-UR.png")
        );

        // Et l'aller-retour dans la même installation est un point fixe.
        let octets = scanflip::ecrire_octets(&application.lignes).unwrap();
        let lecture = scanflip::lire_octets(&octets).unwrap();
        assert!(lecture.application);
        let rapport = import::planifier(
            &lecture.lignes,
            &import::bases_pour(&paths, &lecture.lignes),
        );
        assert!(rapport.refusees.is_empty(), "{:?}", rapport.refusees);
        assert!(rapport.resolues.is_empty() && rapport.redirigees.is_empty());
        let mut rowids: Vec<i64> = rapport.ecritures.iter().map(|e| e.rowid).collect();
        rowids.sort_unstable();
        assert_eq!(rowids, vec![125, 170, 171, 984]);
    }
}
