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
//! # Les classeurs OCG sont écartés
//!
//! Un tirage japonais garde son code natif — `LOCR-JP001` n'a jamais existé
//! en `-FR`. Mais la colonne `Langue` du format, elle, est unique pour tout
//! le fichier : les 154 cartes `LOCR-JP` de l'installation réelle
//! ressortaient étiquetées « Français (France) ». Un code juste sous un
//! libellé faux vaut moins qu'une ligne absente, d'autant que rien ne dit
//! que Scanflip sache lire l'OCG. Les classeurs à suffixe OCG sont donc
//! **écartés**, et [`Bilan`] dit lesquels.
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
use crate::scanflip::{Edition, Sortie};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Sépare les classeurs exportables de ceux que l'OCG écarte.
///
/// Rend `(exportables, ecartes)`, chacun dans l'ordre reçu.
///
/// ```
/// use ygo_app::export::ecarter_ocg;
/// let tout = ["RA05".to_owned(), "LOCR-JP".to_owned(), "SDLI".to_owned()];
/// let (gardes, ecartes) = ecarter_ocg(&tout);
/// assert_eq!(gardes, ["RA05".to_owned(), "SDLI".to_owned()]);
/// assert_eq!(ecartes, ["LOCR-JP".to_owned()]);
/// ```
#[must_use]
pub fn ecarter_ocg(classeurs: &[String]) -> (Vec<String>, Vec<String>) {
    classeurs.iter().cloned().partition(|c| !a_suffixe_ocg(c))
}

/// Ce qu'un export contient, et ce qu'il a laissé de côté.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Lignes écrites.
    pub lignes: usize,
    /// Somme des quantités.
    pub exemplaires: i64,
    /// Lignes dont l'état n'est pas renseigné — elles sortent avec leurs
    /// trois colonnes d'édition vides.
    pub sans_etat: usize,
    /// Classeurs écartés parce qu'OCG.
    pub ecartes: Vec<String>,
    /// Lignes dont le code de rareté n'est pas dans les règles de Scanflip
    /// — elles seront **rejetées** par son import.
    pub raretes_refusees: Vec<String>,
}

/// Compte ce qu'un jeu de lignes contient.
#[must_use]
pub fn bilan(lignes: &[Sortie], ecartes: Vec<String>) -> Bilan {
    Bilan {
        lignes: lignes.len(),
        exemplaires: lignes.iter().map(|l| l.quantite).sum(),
        sans_etat: lignes
            .iter()
            .filter(|l| l.qualite.trim().is_empty())
            .count(),
        ecartes,
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
                langue: langue.libelle().to_owned(),
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
            }
        })
        .collect()
}

/// Lit les cartes possédées d'un classeur.
///
/// Pas de filtre sur l'image : une carte possédée dont l'illustration
/// manque reste une carte de la collection.
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
                    quantite, qualite, edition
               FROM cards
              WHERE possessed = 1
              ORDER BY sort_order, rowid",
        )
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let lignes = requete
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
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    Ok(lignes)
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

    /// Les classeurs OCG sont écartés, les autres passent.
    #[test]
    fn les_classeurs_ocg_sont_ecartes_de_l_export() {
        let tout: Vec<String> = ["RA05", "LOCR-JP", "SDLI", "LOCH-JP", "CROS-KR"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let (gardes, ecartes) = ecarter_ocg(&tout);
        assert_eq!(gardes, vec!["RA05".to_owned(), "SDLI".to_owned()]);
        assert_eq!(
            ecartes,
            vec![
                "LOCR-JP".to_owned(),
                "LOCH-JP".to_owned(),
                "CROS-KR".to_owned()
            ]
        );
        // Un classeur TCG dont le nom contient « JP » sans en être le
        // suffixe n'est pas écarté.
        let (gardes, ecartes) = ecarter_ocg(&["JPEG".to_owned()]);
        assert_eq!(gardes, vec!["JPEG".to_owned()]);
        assert!(ecartes.is_empty());
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
        let b = bilan(&lignes, vec!["LOCR-JP".to_owned()]);
        assert_eq!(b.lignes, 3);
        assert_eq!(b.exemplaires, 4, "2 + 1 + 1");
        assert_eq!(b.sans_etat, 2, "la vide et celle qui n'a que des espaces");
        assert_eq!(b.ecartes, vec!["LOCR-JP".to_owned()]);
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
        let b = bilan(&lignes, Vec::new());
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
}
