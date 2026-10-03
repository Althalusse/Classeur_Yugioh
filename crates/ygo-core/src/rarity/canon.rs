// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Canonisation des libellés de rareté.
//!
//! # Le problème, mesuré et non supposé
//!
//! Sur les 26 classeurs réels de l'utilisateur — 7 136 lignes — **912 lignes
//! (12,8 %) portent une écriture non canonique** :
//!
//! | Écriture | Lignes | Classeurs |
//! |---|---|---|
//! | `PLatinum Secret Rare` | 160 | `RA05` |
//! | `C` | 149 | `LDK2`, `SDLI` |
//! | `UR` | 103 | `RA02`, `EGO1`, `EGS1`, `LDK2`, `SDLI` |
//! | `SR` | 91 | idem |
//! | `ScR` | 84 | `RA02`, `LDK2` |
//! | `CR`, `PlScR`, `QCScR`, `UtR` | 81 chacune | `RA02` |
//! | `force-SMW` | 1 | `RA05` |
//!
//! Ce n'est pas un défaut d'affichage. [`super::Priorites`] rend
//! [`super::PRIORITE_INCONNUE_TRI`] (`9999`) au tri et
//! [`super::PRIORITE_INCONNUE_FILTRE`] (`0`) au filtre pour tout libellé absent
//! de `rarity_config.json` : ces 912 lignes **sont déjà mal triées et mal
//! filtrées**, dans le Python comme dans le portage. `UR` ne se range pas avec
//! `Ultra Rare`, il part en fin de classeur.
//!
//! # La table n'est pas retranscrite, elle est extraite
//!
//! `assets/raretes_alias.json` est produit par
//! `outils/extraire_alias_raretes.py`, qui lit les captures YGOPRODeck réelles.
//! Chaque tirage y porte `set_rarity_code` (« (UR) ») **et** `set_rarity`
//! (« Ultra Rare ») sur la même ligne : personne n'a rapproché les deux
//! champs, la source les livre appariés. Vingt-cinq abréviations, **aucun
//! conflit** — le script échoue s'il en trouve un.
//!
//! Deux abréviations manquaient à l'appel : `PlScR` et `QCScR`, qu'YGOPRODeck
//! n'emploie pas (il écrit `(PS)` et rien du tout). Elles viennent de
//! Yugipedia, qui ne les publie que dans les **noms de fichiers** de ses
//! galeries. Elles vivent à part, dans
//! `assets/raretes_alias_yugipedia.json`, où chaque entrée porte l'URL de la
//! page et le fichier qui l'atteste — la provenance est dans la donnée, pas
//! dans un souvenir.
//!
//! # L'autorité reste la liste des Options
//!
//! La forme canonique n'est pas décidée ici : c'est celle de
//! `rarity_config.json`, la liste que l'utilisateur voit dans les Options.
//! `Collector's Rare` avec l'apostrophe, donc, et non `Collector Rare`.
//!
//! # Un nombre n'est pas une rareté — c'est un nombre d'exemplaires
//!
//! YGOPRODeck rend, pour certains reprints de Structure Deck, un `set_rarity`
//! qui est un **chiffre nu** et un `set_rarity_code` vide :
//!
//! ```text
//! SDWD-EN013  Sage with Eyes of Blue          set_rarity: "3"   ← 3 exemplaires
//! SDWD-EN015  Dictator of D.                  set_rarity: "2"   ← 2 exemplaires
//! ```
//!
//! Le nombre est le `qty` du deck : `cardinfo.db` porte, pour les mêmes
//! tirages, `rarity = Common` **et** `qty = 3` / `qty = 2`. Les six occurrences
//! de `SDWD` sont exactement ses six cartes en plusieurs exemplaires — la
//! correspondance est totale, sur les deux sources, ce qui exclut la
//! coïncidence.
//!
//! Laisser passer ce libellé coûte trois fois : la ligne se range en fin de
//! classeur (`9999` au tri), l'import CSV ne la retrouve jamais — le CSV dit
//! `Common`, la base dit `3` — et la passe artworks, ne trouvant pas de ligne
//! `Common` pour ce numéro, en **insère une seconde**. L'utilisateur voit
//! alors une carte en double dont un exemplaire ne se remplit jamais.
//!
//! Un libellé entièrement numérique vaut donc `Common`. Ce n'est pas un
//! rapprochement approximatif : c'est un champ dont on sait, par recoupement
//! de deux sources, qu'il ne contient pas ce qu'il annonce. Le Python le
//! réparait après coup (`migration_set_codes.migrer_raretes_numeriques`, même
//! défaut, même conclusion) ; ici la règle vit dans la canonisation, donc elle
//! s'applique **avant l'écriture** à la création, et en reprise sur les
//! classeurs déjà en place — un seul endroit, trois usages.
//!
//! # Ce qui n'est jamais deviné
//!
//! `force-SMW` — relevé une fois sur `RA05-EN136`, et cause de la disparition
//! pure et simple de cette carte du classeur Python — n'est **pas** une
//! rareté : c'est une annotation qu'YGOPRODeck a laissée passer dans le champ.
//! Aucun rapprochement approximatif n'est tenté. Un libellé qu'aucune table ne
//! reconnaît est rendu tel quel et signalé. Il n'est pas numérique, et la
//! règle ci-dessus ne l'atteint pas : c'est bien une **forme** reconnaissable
//! qu'on traite, pas un libellé qu'on n'a pas su lire.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::reference::normaliser;
use super::Priorites;

/// La table d'abréviations, telle qu'extraite des captures.
const ALIAS_JSON: &str = include_str!("../../../../assets/raretes_alias.json");

/// Une abréviation et le libellé complet qu'elle désigne.
#[derive(Debug, Clone, Deserialize)]
pub struct Alias {
    /// L'abréviation, telle que la source l'écrit (`UR`, `ScR`…).
    pub abreviation: String,
    /// Le libellé complet correspondant.
    pub canonique: String,
    /// D'où vient l'appariement (`ygoprodeck`, `yugipedia`).
    pub source: String,
    /// Pour une capture manuelle, l'URL et le fichier qui l'attestent.
    #[serde(default)]
    pub preuve: Option<String>,
}

fn table() -> &'static Vec<Alias> {
    static TABLE: OnceLock<Vec<Alias>> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(ALIAS_JSON).unwrap_or_default())
}

fn index_alias() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (i, a) in table().iter().enumerate() {
            index.insert(normaliser(&a.abreviation), i);
        }
        index
    })
}

/// L'index inverse : libellé canonique → abréviation **d'YGOPRODeck**.
///
/// Restreint à `source == "ygoprodeck"`, et c'est la restriction qui le rend
/// utilisable : sur les 25 entrées de cette source, 25 libellés distincts,
/// **aucune collision**. Toutes sources confondues, `Platinum Secret Rare`
/// en porterait deux — `PS` chez YGOPRODeck, `PlScR` chez Yugipedia — et le
/// renversement n'aurait plus de réponse unique.
fn index_inverse() -> &'static HashMap<String, &'static str> {
    static INDEX: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for a in table().iter().filter(|a| a.source == "ygoprodeck") {
            index.insert(normaliser(&a.canonique), a.abreviation.as_str());
        }
        index
    })
}

/// L'abréviation qu'YGOPRODeck emploie pour ce libellé de rareté.
///
/// # À quoi elle sert, et pourquoi elle n'est pas le code Scanflip
///
/// La colonne `rarity_code` d'un classeur porte le `set_rarity_code`
/// d'YGOPRODeck — `ScR` pour Secret Rare, `PS` pour Platinum Secret Rare.
/// [`super::reference::nom_vers_code`] rend, lui, le code **Scanflip** —
/// `SCR` pour la même rareté. Deux systèmes, deux graphies, et aucun n'est le
/// mauvais : le premier décrit ce que la colonne contient, le second ce que
/// le CSV doit dire. Les confondre écrirait un mot d'une langue dans le
/// dictionnaire d'une autre.
///
/// Cette fonction sert à remplir `rarity_code` pour une ligne **créée** par
/// l'application, pour laquelle aucune réponse d'YGOPRODeck n'existe. Elle ne
/// sert pas à réécrire le code d'une ligne existante : celui-là vient de la
/// source, et il est vide exactement là où la source n'a rien donné.
///
/// ```
/// use ygo_core::rarity::canon::abreviation_ygoprodeck;
/// assert_eq!(abreviation_ygoprodeck("Secret Rare"), Some("ScR"));
/// assert_eq!(abreviation_ygoprodeck("Ultra Rare"), Some("UR"));
/// assert_eq!(abreviation_ygoprodeck("Platinum Secret Rare"), Some("PS"));
/// // La casse et les accents ne comptent pas.
/// assert_eq!(abreviation_ygoprodeck("secret rare"), Some("ScR"));
/// // Une rareté qu'YGOPRODeck n'abrège pas — rien plutôt qu'une invention.
/// assert_eq!(abreviation_ygoprodeck("Quarter Century Secret Rare"), None);
/// assert_eq!(abreviation_ygoprodeck("force-SMW"), None);
/// ```
#[must_use]
pub fn abreviation_ygoprodeck(libelle: &str) -> Option<&'static str> {
    index_inverse().get(&normaliser(libelle)).copied()
}

/// Toutes les abréviations connues.
#[must_use]
pub fn alias() -> &'static [Alias] {
    table()
}

/// Par quel chemin un libellé a été ramené à sa forme canonique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origine {
    /// Le libellé figurait déjà, au caractère près, dans la liste des Options.
    Deja,
    /// Retrouvé dans la liste des Options à la casse et aux accents près —
    /// c'est le chemin de `PLatinum Secret Rare`.
    Orthographe,
    /// Abréviation de la table extraite (`UR` → `Ultra Rare`).
    Abreviation,
    /// Alias du référentiel Scanflip (`Grandmaster Rare` → `Grand Master
    /// Rare`). N'est consulté qu'en dernier, et seulement pour un libellé
    /// absent de la liste des Options : `Short Print` y figure et reste donc
    /// distinct de `Common`, dont il est pourtant l'alias.
    Alias,
    /// Libellé entièrement numérique — le `qty` d'YGOPRODeck tombé dans le
    /// champ `set_rarity`. Vaut `Common` (cf. tête de module).
    NombreDExemplaires,
}

/// Ce que vaut un libellé de rareté entièrement numérique.
///
/// Les six occurrences relevées sont des reprints de Structure Deck, que
/// `cardinfo.db` donne tous en `Common`. Le Python retenait la même valeur,
/// pour la même raison.
pub const RARETE_DES_NOMBRES: &str = "Common";

/// Ce libellé n'est-il fait que de chiffres ?
///
/// La question est posée sur le libellé **entier** : `10000 Secret Rare` et
/// `20th Secret Rare` sont de vraies raretés qui commencent par un chiffre, et
/// aucune des deux ne doit être touchée.
///
/// ```
/// use ygo_core::rarity::canon::est_nombre_d_exemplaires;
/// assert!(est_nombre_d_exemplaires("3"));
/// assert!(est_nombre_d_exemplaires(" 2 "));
/// assert!(!est_nombre_d_exemplaires("10000 Secret Rare"));
/// assert!(!est_nombre_d_exemplaires("20th Secret Rare"));
/// assert!(!est_nombre_d_exemplaires("force-SMW"));
/// assert!(!est_nombre_d_exemplaires(""));
/// ```
#[must_use]
pub fn est_nombre_d_exemplaires(libelle: &str) -> bool {
    let libelle = libelle.trim();
    !libelle.is_empty() && libelle.chars().all(|c| c.is_ascii_digit())
}

/// Le résultat d'une canonisation réussie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canon {
    /// La forme canonique.
    pub libelle: String,
    /// Le chemin emprunté.
    pub origine: Origine,
}

impl Canon {
    /// Le libellé était-il déjà canonique ? Rien à réécrire, dans ce cas.
    #[must_use]
    pub fn inchange(&self, libelle: &str) -> bool {
        self.libelle == libelle
    }
}

/// Ramène un libellé de rareté à sa forme canonique.
///
/// `reference` est la liste des Options — `rarity_config.json` — qui fait
/// autorité sur la forme retenue.
///
/// Quatre passes, dans cet ordre :
///
/// 1. correspondance **exacte** dans la référence ;
/// 2. correspondance **normalisée** dans la référence (casse, accents,
///    espaces, apostrophes et tirets ignorés) ;
/// 3. **abréviation** de la table extraite ;
/// 4. **alias** du référentiel Scanflip.
///
/// Rend `None` quand rien ne correspond — jamais une approximation.
///
/// ```
/// use ygo_core::rarity::canon::{canoniser, Origine};
/// use ygo_core::rarity::Priorites;
///
/// let options = Priorites::depuis_paires([("Ultra Rare", 4), ("Platinum Secret Rare", 7)]);
/// assert_eq!(canoniser("UR", &options).unwrap().libelle, "Ultra Rare");
/// assert_eq!(
///     canoniser("PLatinum Secret Rare", &options).unwrap().origine,
///     Origine::Orthographe
/// );
/// assert!(canoniser("force-SMW", &options).is_none());
/// ```
#[must_use]
pub fn canoniser(libelle: &str, reference: &Priorites) -> Option<Canon> {
    let libelle = libelle.trim();
    if libelle.is_empty() {
        return None;
    }

    if reference.brute(libelle).is_some() {
        return Some(Canon {
            libelle: libelle.to_owned(),
            origine: Origine::Deja,
        });
    }

    // Avant toute table : un libellé qui n'est fait que de chiffres n'est pas
    // une rareté mais un nombre d'exemplaires. Cf. tête de module.
    if est_nombre_d_exemplaires(libelle) {
        return Some(Canon {
            libelle: RARETE_DES_NOMBRES.to_owned(),
            origine: Origine::NombreDExemplaires,
        });
    }

    let cle = normaliser(libelle);
    if let Some((nom, _)) = reference.iter().find(|(nom, _)| normaliser(nom) == cle) {
        return Some(Canon {
            libelle: nom.to_owned(),
            origine: Origine::Orthographe,
        });
    }

    if let Some(a) = index_alias().get(&cle).and_then(|i| table().get(*i)) {
        return Some(Canon {
            libelle: a.canonique.clone(),
            origine: Origine::Abreviation,
        });
    }

    // Dernier recours : le référentiel Scanflip, dont les alias sont plus
    // larges (« Short Print » y vaut « Common »). Il n'est atteint que par un
    // libellé que la liste des Options ne connaît sous aucune orthographe.
    let code = crate::rarity::reference::nom_vers_code(libelle)?;
    let nom = super::reference::code_vers_nom_en(code);
    // Un libellé qui EST son propre nom canonique est reconnu, pas rejeté.
    //
    // Ce `if` rendait `None` — au motif d'éviter une canonisation qui ne
    // change rien. Or ne rien changer est exactement ce que produit un
    // libellé **déjà juste** : toute rareté présente dans
    // `raretes_reference.json` mais absente du `rarity_config.json` de
    // l'utilisateur était donc déclarée inconnue, alors que le référentiel la
    // connaît. Mesuré le 2026-09-05 sur les classeurs réels : `Grand Master
    // Rare` (entrée 37, code `GMR`) échouait sur **35 lignes** de `LOCH-JP` et
    // `LOCR-JP` — tombant sur `PRIORITE_INCONNUE_TRI` au tri et `0` au
    // filtre — là où sa faute d'orthographe `Grandmaster Rare` et son
    // abréviation `GMR` passaient toutes deux.
    let origine = if nom == libelle {
        Origine::Deja
    } else {
        Origine::Alias
    };
    Some(Canon {
        libelle: nom,
        origine,
    })
}

/// Clé de comparaison d'une rareté, **canonisée**.
///
/// # Pourquoi cette fonction existe
///
/// La passe artworks compare la rareté que Yugipedia écrit à celle du
/// classeur, via `normaliser_rarete` — minuscules, caractères non
/// alphanumériques ôtés. Cette clé-là ne rapproche **jamais** `PlScR` de
/// `Platinum Secret Rare` : `plscr` et `platinumsecretrare` sont deux chaînes
/// différentes. La passe en conclut que le tirage manque au classeur, et elle
/// l'insère.
///
/// C'est l'origine, mesurée sur les bases réelles, de 751 lignes en double :
/// 567 sur `RA02`, 132 sur `LDK2`, 36 sur `SDLI`, 8 sur `EGO1` et 8 sur
/// `EGS1` — toutes à quantité nulle, avec un `sort_order` recopié de la
/// première ligne du groupe et une `edition` vide.
///
/// Canoniser **avant** de normaliser referme le trou : les deux écritures
/// donnent la même clé, la ligne existante est retrouvée, rien n'est inséré.
///
/// Un libellé qu'aucune table ne reconnaît garde sa clé brute — il ne doit pas
/// se confondre avec une rareté voisine.
///
/// ```
/// use ygo_core::rarity::canon::cle;
/// use ygo_core::rarity::Priorites;
///
/// let options = Priorites::depuis_paires([("Platinum Secret Rare", 7)]);
/// assert_eq!(cle("PlScR", &options), cle("Platinum Secret Rare", &options));
/// assert_eq!(cle("PlScR", &options), "platinumsecretrare");
/// // Ce qui n'est pas une rareté ne rejoint personne.
/// assert_eq!(cle("force-SMW", &options), "forcesmw");
/// ```
#[must_use]
pub fn cle(libelle: &str, reference: &Priorites) -> String {
    let canonique = canoniser(libelle, reference).map_or_else(|| libelle.to_owned(), |c| c.libelle);
    canonique
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    /// Le `rarity_config.json` réel de l'utilisateur — la liste des Options.
    fn options() -> Priorites {
        const REFERENCE: &str =
            include_str!("../../../../tests/fixtures/rarity_config.reference.json");
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(&chemin, REFERENCE).unwrap();
        Priorites::charger(&chemin)
    }

    #[test]
    fn la_table_extraite_se_charge_sans_conflit() {
        assert_eq!(alias().len(), 27);
        let mut cles: Vec<String> = alias().iter().map(|a| normaliser(&a.abreviation)).collect();
        cles.sort();
        let avant = cles.len();
        cles.dedup();
        assert_eq!(cles.len(), avant, "aucune abréviation en collision");
        assert_eq!(
            alias().iter().filter(|a| a.source == "ygoprodeck").count(),
            25
        );
    }

    /// Les huit écritures relevées dans les classeurs réels, une à une.
    #[test]
    fn les_abreviations_reelles_sont_toutes_resolues() {
        let o = options();
        for (ecrit, attendu) in [
            ("C", "Common"),
            ("UR", "Ultra Rare"),
            ("SR", "Super Rare"),
            ("ScR", "Secret Rare"),
            ("CR", "Collector's Rare"),
            ("UtR", "Ultimate Rare"),
        ] {
            let c = canoniser(ecrit, &o).unwrap_or_else(|| panic!("{ecrit} non résolu"));
            assert_eq!(c.libelle, attendu, "{ecrit}");
            assert_eq!(c.origine, Origine::Abreviation, "{ecrit}");
        }
    }

    /// `PlScR` et `QCScR` (162 lignes de `RA02`) ne sont **pas** du vocabulaire
    /// YGOPRODeck, qui écrit `(PS)` et rien du tout. Ils viennent des noms de
    /// fichiers de Yugipedia et sont capturés à part, avec leur preuve.
    #[test]
    fn les_abreviations_yugipedia_sont_capturees_a_part() {
        let o = options();
        assert_eq!(
            canoniser("PlScR", &o).unwrap().libelle,
            "Platinum Secret Rare"
        );
        assert_eq!(
            canoniser("QCScR", &o).unwrap().libelle,
            "Quarter Century Secret Rare"
        );
        // Le vocabulaire YGOPRODeck pour la même rareté cohabite sans conflit.
        assert_eq!(canoniser("PS", &o).unwrap().libelle, "Platinum Secret Rare");

        // Toute entrée non-YGOPRODeck doit porter la trace de sa provenance :
        // c'est ce qui distingue une capture d'une retranscription.
        for a in alias().iter().filter(|a| a.source != "ygoprodeck") {
            assert!(
                a.preuve.as_ref().is_some_and(|p| p.starts_with("https://")),
                "{} sans preuve",
                a.abreviation
            );
        }
    }

    #[test]
    fn la_coquille_de_ra05_se_corrige_par_l_orthographe() {
        let o = options();
        let c = canoniser("PLatinum Secret Rare", &o).unwrap();
        assert_eq!(c.libelle, "Platinum Secret Rare");
        assert_eq!(c.origine, Origine::Orthographe);
        // Et la variante « PLatium », relevée sur le wiki, n'est reconnue par
        // aucune passe : une lettre manque, ce n'est plus la même chaîne.
        assert!(canoniser("PLatium Secret Rare", &o).is_none());
    }

    #[test]
    fn un_libelle_deja_canonique_ne_bouge_pas() {
        let o = options();
        for nom in [
            "Ultra Rare",
            "Collector's Rare",
            "Quarter Century Secret Rare",
        ] {
            let c = canoniser(nom, &o).unwrap();
            assert_eq!(c.origine, Origine::Deja);
            assert!(c.inchange(nom));
        }
    }

    /// L'apostrophe est la forme de la liste des Options, et c'est elle qui
    /// gagne — y compris contre une saisie sans apostrophe.
    ///
    /// # Une limite assumée
    ///
    /// `Collector Rare`, sans le `s`, n'est **pas** résolu. La normalisation
    /// ôte l'apostrophe, pas une lettre : `collectorrare` et `collectorsrare`
    /// restent deux chaînes différentes. C'est le prix du refus de
    /// l'approximation — celui-là même qui empêche `force-SMW` d'être rattaché
    /// à une rareté au hasard.
    #[test]
    fn l_autorite_est_la_liste_des_options() {
        let o = options();
        assert_eq!(
            canoniser("collectors rare", &o).unwrap().libelle,
            "Collector's Rare"
        );
        assert_eq!(
            canoniser("COLLECTOR'S RARE", &o).unwrap().libelle,
            "Collector's Rare"
        );
        assert!(
            canoniser("Collector Rare", &o).is_none(),
            "une lettre manque"
        );
    }

    /// Le piège du référentiel Scanflip : `Short Print` y est un **alias** de
    /// `Common`. Mais il figure dans la liste des Options comme rareté à part
    /// entière, priorité 13. La première passe le retient donc tel quel, et la
    /// passe d'alias n'est jamais atteinte.
    #[test]
    fn short_print_reste_short_print() {
        let o = options();
        assert_eq!(o.brute("Short Print"), Some(13));
        let c = canoniser("Short Print", &o).unwrap();
        assert_eq!(c.libelle, "Short Print");
        assert_eq!(c.origine, Origine::Deja);
    }

    /// La passe d'alias n'est atteinte que par un libellé qu'aucune
    /// orthographe de la liste des Options ne recouvre.
    ///
    /// `Grandmaster Rare` **n'y arrive pas** : normalisé, il vaut
    /// `grandmasterrare`, exactement comme `Grand Master Rare` de la liste — la
    /// passe d'orthographe le prend au vol. Il faut un libellé dont la forme
    /// complète diffère vraiment, comme `Starfoil` face à `Starfoil Rare`.
    #[test]
    fn la_passe_d_alias_ne_sert_qu_aux_absents_de_la_liste() {
        let o = options();

        assert_eq!(o.brute("Grandmaster Rare"), None);
        let par_orthographe = canoniser("Grandmaster Rare", &o).unwrap();
        assert_eq!(par_orthographe.libelle, "Grand Master Rare");
        assert_eq!(par_orthographe.origine, Origine::Orthographe);

        assert_eq!(o.brute("Starfoil"), None);
        let par_alias = canoniser("Starfoil", &o).unwrap();
        assert_eq!(par_alias.libelle, "Starfoil Rare");
        assert_eq!(par_alias.origine, Origine::Alias);

        assert_eq!(o.brute("Extra Secret"), None);
        assert_eq!(
            canoniser("Extra Secret", &o).unwrap().libelle,
            "Extra Secret Rare"
        );
    }

    /// Les sept écritures de `RA02` doivent toutes rejoindre leur libellé
    /// complet — c'est la condition pour que la passe artworks cesse d'insérer
    /// des doublons.
    #[test]
    fn la_cle_rapproche_les_deux_vocabulaires() {
        let o = options();
        for (abrege, complet) in [
            ("CR", "Collector's Rare"),
            ("PlScR", "Platinum Secret Rare"),
            ("QCScR", "Quarter Century Secret Rare"),
            ("ScR", "Secret Rare"),
            ("SR", "Super Rare"),
            ("UR", "Ultra Rare"),
            ("UtR", "Ultimate Rare"),
        ] {
            assert_eq!(cle(abrege, &o), cle(complet, &o), "{abrege} / {complet}");
        }
        // La coquille de RA05 rejoint aussi la bonne clé.
        assert_eq!(
            cle("PLatinum Secret Rare", &o),
            cle("Platinum Secret Rare", &o)
        );
    }

    /// Deux raretés distinctes gardent des clés distinctes : la canonisation ne
    /// doit pas fusionner ce qui doit rester séparé.
    #[test]
    fn la_cle_ne_confond_pas_deux_raretes() {
        let o = options();
        let distinctes = [
            "Common",
            "Short Print",
            "Secret Rare",
            "Platinum Secret Rare",
            "Quarter Century Secret Rare",
            "Ultra Rare",
            "Ultimate Rare",
        ];
        let mut cles: Vec<String> = distinctes.iter().map(|r| cle(r, &o)).collect();
        cles.sort();
        let avant = cles.len();
        cles.dedup();
        assert_eq!(cles.len(), avant);
    }

    /// Un libellé non reconnu garde sa clé brute — il ne rejoint personne.
    #[test]
    fn la_cle_d_un_inconnu_reste_la_sienne() {
        let o = options();
        assert_eq!(cle("force-SMW", &o), "forcesmw");
        assert_ne!(cle("force-SMW", &o), cle("Secret Rare", &o));
    }

    /// Le renversement de la table est **complet et sans collision** — et
    /// c'est la restriction à YGOPRODeck qui le rend possible.
    #[test]
    fn le_renversement_de_la_table_est_sans_ambiguite() {
        // Chaque abréviation d'YGOPRODeck se retrouve depuis son libellé.
        let ygoprodeck: Vec<&Alias> = alias()
            .iter()
            .filter(|a| a.source == "ygoprodeck")
            .collect();
        assert_eq!(ygoprodeck.len(), 25);
        for a in &ygoprodeck {
            assert_eq!(
                abreviation_ygoprodeck(&a.canonique),
                Some(a.abreviation.as_str()),
                "{} → {}",
                a.canonique,
                a.abreviation
            );
        }
        // Et aucun libellé n'en porte deux.
        let libelles: std::collections::HashSet<String> = ygoprodeck
            .iter()
            .map(|a| crate::rarity::reference::normaliser(&a.canonique))
            .collect();
        assert_eq!(libelles.len(), ygoprodeck.len(), "aucune collision");

        // `Platinum Secret Rare` en porte deux toutes sources confondues —
        // c'est celle d'YGOPRODeck qui sort, parce que c'est celle que la
        // colonne `rarity_code` parle.
        assert_eq!(abreviation_ygoprodeck("Platinum Secret Rare"), Some("PS"));
        assert!(
            alias()
                .iter()
                .any(|a| a.abreviation == "PlScR" && a.source == "yugipedia"),
            "l'autre existe bien, elle vient de Yugipedia"
        );
    }

    /// Les deux systèmes de codes divergent sur **21 raretés sur 25**.
    ///
    /// # Pourquoi ce chiffre est dans un test
    ///
    /// `reference::nom_vers_code` dit ce que **Scanflip** attend dans un CSV ;
    /// `abreviation_ygoprodeck` dit ce que la colonne `rarity_code` d'un
    /// classeur contient. On pourrait croire à deux graphies d'une même
    /// chose : `Ultra Rare` est `UR` chez YGOPRODeck et **`U`** chez
    /// Scanflip, `Collector's Rare` est `CR` contre `COL`, `Platinum Secret
    /// Rare` est `PS` contre `SPL`. Quatre seulement coïncident.
    ///
    /// Prendre l'un pour l'autre ne se verrait donc pas quatre fois sur cinq,
    /// mais écrirait un code faux. Le test fige le partage pour qu'un futur
    /// « unifions ces deux tables » se heurte à la mesure.
    #[test]
    fn les_deux_systemes_de_codes_divergent_sur_la_plupart_des_raretes() {
        let (mut identiques, mut differents) = (0, 0);
        for a in alias().iter().filter(|a| a.source == "ygoprodeck") {
            match (
                abreviation_ygoprodeck(&a.canonique),
                crate::rarity::reference::nom_vers_code(&a.canonique),
            ) {
                (Some(y), Some(s)) if y == s => identiques += 1,
                (Some(_), Some(_)) => differents += 1,
                (_, autre) => panic!("{} : Scanflip ne connaît pas ({autre:?})", a.canonique),
            }
        }
        assert_eq!((identiques, differents), (4, 21));

        // Les trois écarts les plus faciles à prendre l'un pour l'autre.
        for (libelle, ygoprodeck, scanflip) in [
            ("Secret Rare", "ScR", "SCR"),
            ("Ultra Rare", "UR", "U"),
            ("Collector's Rare", "CR", "COL"),
        ] {
            assert_eq!(abreviation_ygoprodeck(libelle), Some(ygoprodeck));
            assert_eq!(
                crate::rarity::reference::nom_vers_code(libelle),
                Some(scanflip)
            );
        }
    }

    #[test]
    fn rien_n_est_devine() {
        let o = options();
        assert!(
            canoniser("force-SMW", &o).is_none(),
            "une annotation, pas une rareté"
        );
        assert!(canoniser("", &o).is_none());
        assert!(canoniser("   ", &o).is_none());
        assert!(canoniser("New artwork", &o).is_none());
        assert!(canoniser("Reprint", &o).is_none());
    }

    /// Un chiffre nu vaut `Common` — et c'est la seule chose que la
    /// canonisation accepte de conclure d'un libellé qui n'est pas une
    /// rareté.
    ///
    /// # Pourquoi ce test a changé de camp
    ///
    /// Il exigeait autrefois `canoniser("2").is_none()`, au nom de « rien
    /// n'est deviné ». Les données ont tranché : sur `SDWD`, YGOPRODeck rend
    /// `set_rarity = "3"` pour *Sage with Eyes of Blue* et `"2"` pour les cinq
    /// autres cartes du deck en plusieurs exemplaires, quand `cardinfo.db`
    /// donne pour les mêmes tirages `rarity = Common` et `qty = 3` / `qty = 2`.
    /// La correspondance est exacte sur les six, et sur les deux sources.
    ///
    /// Ce n'est donc pas une devinette mais la lecture d'un champ dont on sait
    /// ce qu'il contient réellement. Laisser passer le chiffre coûtait trois
    /// fois : tri en fin de classeur, import qui ne retrouve jamais la carte,
    /// et ligne en double créée par la passe artworks.
    #[test]
    fn un_chiffre_nu_est_un_nombre_d_exemplaires_pas_une_rarete() {
        let o = options();
        for chiffre in ["2", "3", " 3 ", "12"] {
            let c = canoniser(chiffre, &o).unwrap_or_else(|| panic!("« {chiffre} »"));
            assert_eq!(c.libelle, "Common");
            assert_eq!(c.origine, Origine::NombreDExemplaires);
        }
        // Les raretés qui commencent par un chiffre ne sont pas touchées.
        for vraie in ["10000 Secret Rare", "20th Secret Rare"] {
            assert_ne!(
                canoniser(vraie, &o).map(|c| c.origine),
                Some(Origine::NombreDExemplaires),
                "« {vraie} » est une vraie rareté"
            );
        }
        // Et un chiffre rejoint bien le groupe des Common : c'est ce qui
        // permet au dédoublonnage de reconnaître son témoin.
        assert_eq!(cle("3", &o), cle("Common", &o));
    }

    /// Sans liste des Options — `rarity_config.json` absent, cas que
    /// [`Priorites::charger`] traite en rendant une table vide — la
    /// canonisation continue de fonctionner : les passes 3 et 4 ne dépendent
    /// pas d'elle. La coquille de `RA05` passe alors par le référentiel
    /// Scanflip au lieu de la liste des Options, et arrive au même endroit.
    #[test]
    fn sans_liste_des_options_les_abreviations_marchent_encore() {
        let vide = Priorites::default();
        assert!(vide.is_empty());

        let abrege = canoniser("UR", &vide).unwrap();
        assert_eq!(abrege.libelle, "Ultra Rare");
        assert_eq!(abrege.origine, Origine::Abreviation);

        let coquille = canoniser("PLatinum Secret Rare", &vide).unwrap();
        assert_eq!(coquille.libelle, "Platinum Secret Rare");
        assert_eq!(
            coquille.origine,
            Origine::Alias,
            "faute de liste, par le référentiel"
        );

        // Ce qui n'est une rareté nulle part ne l'est toujours pas.
        assert!(canoniser("force-SMW", &vide).is_none());
    }

    /// Le défaut du 2026-09-05 : un nom canonique rejeté là où sa faute
    /// d'orthographe passait.
    ///
    /// `Grand Master Rare` est l'entrée 37 du référentiel, code `GMR`. Le
    /// dernier recours rendait `None` dès que la canonisation ne changeait
    /// rien — c'est-à-dire précisément quand le libellé était déjà juste.
    #[test]
    fn un_nom_canonique_absent_des_options_reste_reconnu() {
        let sans = Priorites::depuis_paires([("Common", 1)]);

        let exact = canoniser("Grand Master Rare", &sans).expect("le nom exact est reconnu");
        assert_eq!(exact.libelle, "Grand Master Rare");
        assert_eq!(exact.origine, Origine::Deja);
        assert!(exact.inchange("Grand Master Rare"), "rien à réécrire");

        // Ce qui passait déjà passe toujours, et vers la même cible.
        for variante in ["Grandmaster Rare", "GMR"] {
            let c = canoniser(variante, &sans).expect(variante);
            assert_eq!(c.libelle, "Grand Master Rare");
            assert_eq!(c.origine, Origine::Alias);
        }
    }

    /// La propriété générale, sur les 44 entrées du référentiel : un nom
    /// canonique n'est jamais rejeté, même quand la liste des Options est
    /// quasi vide — le cas d'une installation neuve.
    #[test]
    fn aucun_nom_du_referentiel_n_est_rejete() {
        let minimal = Priorites::depuis_paires([("Common", 1)]);
        let rejetes: Vec<&str> = crate::rarity::reference::table()
            .iter()
            .filter(|r| canoniser(&r.en, &minimal).is_none())
            .map(|r| r.en.as_str())
            .collect();
        assert!(rejetes.is_empty(), "noms canoniques rejetés : {rejetes:?}");
    }

    /// La nuance que la correction ne doit PAS effacer : c'est la liste des
    /// Options qui décide si `Short Print` reste distinct de `Common`.
    ///
    /// `Short Print` n'est pas au référentiel des raretés — il n'existe que
    /// comme alias de `Common` chez Scanflip. Deux comportements, tous deux
    /// voulus et antérieurs à la correction du 2026-09-05 :
    #[test]
    fn short_print_suit_la_liste_des_options() {
        // Déclaré dans les Options : il s'y trouve tel quel, et rien ne bouge.
        let avec = Priorites::depuis_paires([("Common", 1), ("Short Print", 13)]);
        let c = canoniser("Short Print", &avec).expect("déclaré");
        assert_eq!(c.libelle, "Short Print");
        assert_eq!(c.origine, Origine::Deja);

        // Absent des Options : le dernier recours Scanflip le rend à `Common`,
        // dont il est l'alias. C'est une canonisation, pas un rejet.
        let sans = Priorites::depuis_paires([("Common", 1)]);
        let c = canoniser("Short Print", &sans).expect("alias Scanflip");
        assert_eq!(c.libelle, "Common");
        assert_eq!(c.origine, Origine::Alias);
    }
}
