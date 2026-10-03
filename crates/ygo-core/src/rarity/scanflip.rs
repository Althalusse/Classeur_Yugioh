// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Ce que l'import de Scanflip accepte — d'après leur page d'aide.
//!
//! # Deux référentiels, deux autorités
//!
//! [`super::reference`] porte la table du Python V1.0.4 : elle fait autorité
//! sur ce que **notre base stocke**, puisque c'est elle qui a produit les
//! libellés qui s'y trouvent.
//!
//! Ce module-ci porte les règles publiées par Scanflip : elles font autorité
//! sur ce que **leur import accepte**, donc sur ce que notre export a le
//! droit d'écrire, et sur ce que notre import doit savoir relire.
//!
//! Les deux listes comptent quarante-quatre codes et se recouvrent à
//! quarante-trois près. Les deux écarts sont réels :
//!
//! - **`STRB`** (« Starlight Blasonnée ») existe chez Scanflip et manquait
//!   à la table du Python. Un CSV la nommant était refusé par notre import.
//! - **`GMR`** (« Grand Master Rare ») existe dans la table du Python et
//!   **pas** chez Scanflip. C'est une rareté OCG, que ce site francophone
//!   consacré au TCG ne connaît pas. Notre export l'écrivait sans savoir
//!   qu'elle serait rejetée à l'autre bout ; il le dit désormais.
//!
//! # Les écritures tolérées
//!
//! Scanflip n'exige pas le code : il accepte aussi le libellé complet, et
//! quelques formes courtes — « UR » ou « Ultra » pour Ultra Rare, « S » ou
//! « Super » pour Super Rare, « Secret » pour Secrète Rare. La règle
//! générale qu'ils énoncent est que le suffixe « Rare » est facultatif pour
//! toute rareté dont le nom se termine par ce mot ; [`code_de`] l'applique
//! en réessayant avec le suffixe quand la première recherche échoue.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// Les règles, telles qu'`outils/extraire_regles_scanflip.py` les fige.
const REGLES_JSON: &str = include_str!("../../../../assets/raretes_scanflip.json");

/// Une rareté que Scanflip accepte.
#[derive(Debug, Clone, Deserialize)]
pub struct Rarete {
    /// Le code court (`SCR`, `QCR`…).
    pub code: String,
    /// Le libellé français, tel que Scanflip l'écrit.
    pub fr: String,
}

#[derive(Debug, Deserialize)]
struct Regles {
    raretes: Vec<Rarete>,
    /// Chaque ligne : le code court d'abord, puis les autres écritures.
    qualites: Vec<Vec<String>>,
}

struct Index {
    regles: Regles,
    /// Toute écriture normalisée d'une rareté → son code.
    par_ecriture: HashMap<String, usize>,
    /// Toute écriture normalisée d'une qualité → son code court.
    qualites: HashMap<String, String>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let regles: Regles = serde_json::from_str(REGLES_JSON).unwrap_or(Regles {
            raretes: Vec::new(),
            qualites: Vec::new(),
        });
        let mut par_ecriture = HashMap::new();
        for (i, r) in regles.raretes.iter().enumerate() {
            par_ecriture.insert(super::reference::normaliser(&r.code), i);
            par_ecriture.insert(super::reference::normaliser(&r.fr), i);
        }
        let mut qualites = HashMap::new();
        for ligne in &regles.qualites {
            let Some(code) = ligne.first() else { continue };
            for ecriture in ligne {
                qualites.insert(super::reference::normaliser(ecriture), code.clone());
            }
        }
        Index {
            regles,
            par_ecriture,
            qualites,
        }
    })
}

/// Les raretés que Scanflip accepte, dans l'ordre de sa page d'aide.
#[must_use]
pub fn table() -> &'static [Rarete] {
    &index().regles.raretes
}

/// Ce code est-il accepté par l'import de Scanflip ?
///
/// Sert à l'export : écrire un code que Scanflip refuse produit une ligne
/// perdue en silence à l'autre bout.
///
/// ```
/// use ygo_core::rarity::scanflip::code_accepte;
/// assert!(code_accepte("QCR"));
/// assert!(code_accepte("STRB"));
/// // Rareté OCG : la table du Python la connaît, Scanflip non.
/// assert!(!code_accepte("GMR"));
/// ```
#[must_use]
pub fn code_accepte(code: &str) -> bool {
    index()
        .regles
        .raretes
        .iter()
        .any(|r| r.code.eq_ignore_ascii_case(code.trim()))
}

/// Le code Scanflip d'une écriture quelconque de rareté.
///
/// Accepte le code, le libellé français, et — comme Scanflip — le libellé
/// amputé de son « Rare » final.
///
/// ```
/// use ygo_core::rarity::scanflip::code_de;
/// assert_eq!(code_de("SCR"), Some("SCR"));
/// assert_eq!(code_de("Secrète Rare"), Some("SCR"));
/// assert_eq!(code_de("secrete rare"), Some("SCR"));
/// // Le « Rare » final est facultatif.
/// assert_eq!(code_de("Ultra"), Some("U"));
/// assert_eq!(code_de("Super"), Some("SR"));
/// assert_eq!(code_de("bof"), None);
/// ```
#[must_use]
pub fn code_de(ecriture: &str) -> Option<&'static str> {
    let idx = index();
    let normalise = super::reference::normaliser(ecriture);
    if normalise.is_empty() {
        return None;
    }
    let i = idx.par_ecriture.get(&normalise).or_else(|| {
        // « Ultra » vaut « Ultra Rare » : Scanflip rend le suffixe
        // facultatif pour toute rareté dont le nom s'y termine.
        idx.par_ecriture.get(&format!("{normalise}rare"))
    })?;
    idx.regles.raretes.get(*i).map(|r| r.code.as_str())
}

/// Le code court d'un état de conservation, quelle qu'en soit l'écriture.
///
/// ```
/// use ygo_core::rarity::scanflip::qualite_de;
/// assert_eq!(qualite_de("NM"), Some("NM"));
/// assert_eq!(qualite_de("NM+"), Some("NM"));
/// assert_eq!(qualite_de("Near Mint"), Some("NM"));
/// assert_eq!(qualite_de("Damaged"), Some("DM"));
/// assert_eq!(qualite_de("parfait"), None);
/// ```
#[must_use]
pub fn qualite_de(ecriture: &str) -> Option<&'static str> {
    index()
        .qualites
        .get(&super::reference::normaliser(ecriture))
        .map(String::as_str)
}

/// Le guide de notation de Scanflip, recopié tel quel.
const GUIDE_ETATS: &str = include_str!("../../../../assets/scanflip_etats.txt");

/// Un état de conservation, tel que le guide de notation de Scanflip le
/// définit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Etat {
    /// Le code court — `NM`.
    pub code: String,
    /// Le nom du grade — `Near Mint`.
    pub nom: String,
    /// La phrase d'ouverture de la section, verbatim.
    pub phrase: String,
    /// Les critères, dans l'ordre de la page.
    pub criteres: Vec<String>,
    /// Les défauts tolérés, quand le guide en cite.
    pub toleres: Vec<String>,
    /// Les remarques qui ferment la section.
    pub remarques: Vec<String>,
}

impl Etat {
    /// La phrase d'ouverture, sans les deux-points qui annoncent la liste —
    /// pour la lire seule.
    #[must_use]
    pub fn resume(&self) -> &str {
        self.phrase.trim_end().trim_end_matches(':').trim_end()
    }

    /// La section entière, mise en forme pour une infobulle.
    #[must_use]
    pub fn detail(&self) -> String {
        let mut lignes = vec![format!("{} — {}", self.code, self.nom), self.phrase.clone()];
        lignes.extend(self.criteres.iter().map(|c| format!("  • {c}")));
        if !self.toleres.is_empty() {
            lignes.push("Tolérés :".to_owned());
            lignes.extend(self.toleres.iter().map(|t| format!("  • {t}")));
        }
        lignes.extend(self.remarques.iter().cloned());
        lignes.join("\n")
    }
}

/// Lit le guide recopié. Une ligne mal formée est ignorée plutôt que de
/// faire échouer l'application : un test vérifie, lui, que les sept y sont.
fn lire_guide(texte: &str) -> Vec<Etat> {
    let mut etats: Vec<Etat> = Vec::new();
    for ligne in texte.lines().map(str::trim) {
        if ligne.is_empty() || ligne.starts_with('#') {
            continue;
        }
        if let Some(entete) = ligne.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let (code, nom) = entete.split_once('|').unwrap_or((entete, entete));
            etats.push(Etat {
                code: code.trim().to_owned(),
                nom: nom.trim().to_owned(),
                phrase: String::new(),
                criteres: Vec::new(),
                toleres: Vec::new(),
                remarques: Vec::new(),
            });
            continue;
        }
        let Some(etat) = etats.last_mut() else {
            continue;
        };
        if let Some(c) = ligne.strip_prefix("- ") {
            etat.criteres.push(c.to_owned());
        } else if let Some(t) = ligne.strip_prefix("+ ") {
            etat.toleres.push(t.to_owned());
        } else if let Some(r) = ligne.strip_prefix("> ") {
            etat.remarques.push(r.to_owned());
        } else if etat.phrase.is_empty() {
            etat.phrase = ligne.to_owned();
        }
    }
    etats
}

/// Les états du guide, dans l'ordre de la page — du meilleur au pire.
#[must_use]
pub fn guide_etats() -> &'static [Etat] {
    static GUIDE: OnceLock<Vec<Etat>> = OnceLock::new();
    GUIDE.get_or_init(|| lire_guide(GUIDE_ETATS))
}

/// L'état du guide pour une écriture quelconque.
///
/// Accepte ce qu'accepte [`qualite_de`], plus le nom du guide lui-même —
/// `Played`, que la page d'aide de l'import écrit `Lightly Played`.
///
/// ```
/// use ygo_core::rarity::scanflip::etat;
/// assert_eq!(etat("NM").unwrap().nom, "Near Mint");
/// assert_eq!(etat("Lightly Played").unwrap().code, "PL");
/// assert_eq!(etat("played").unwrap().code, "PL");
/// assert!(etat("parfait").is_none());
/// ```
#[must_use]
pub fn etat(ecriture: &str) -> Option<&'static Etat> {
    let guide = guide_etats();
    let cle = super::reference::normaliser(ecriture);
    if let Some(e) = guide
        .iter()
        .find(|e| super::reference::normaliser(&e.nom) == cle)
    {
        return Some(e);
    }
    let code = qualite_de(ecriture)?;
    guide.iter().find(|e| e.code == code)
}

/// Le nom du grade selon Scanflip — `NM` → `Near Mint`, `PL` → `Played`.
///
/// ```
/// use ygo_core::rarity::scanflip::nom_qualite;
/// assert_eq!(nom_qualite("NM"), Some("Near Mint"));
/// assert_eq!(nom_qualite("nm+"), Some("Near Mint"));
/// assert_eq!(nom_qualite("PL"), Some("Played"));
/// assert_eq!(nom_qualite("DM"), Some("Damaged"));
/// assert_eq!(nom_qualite("parfait"), None);
/// ```
#[must_use]
pub fn nom_qualite(ecriture: &str) -> Option<&'static str> {
    etat(ecriture).map(|e| e.nom.as_str())
}

/// L'état tel qu'on l'affiche : le code, puis son nom entre parenthèses.
///
/// Le code reste devant : c'est lui qui est stocké, exporté, et que Scanflip
/// relit. Le nom n'est là que pour qu'on sache ce qu'on choisit. Une valeur
/// que le guide ne connaît pas passe telle quelle, plutôt que de disparaître.
///
/// ```
/// use ygo_core::rarity::scanflip::libelle_qualite;
/// assert_eq!(libelle_qualite("NM"), "NM (Near Mint)");
/// assert_eq!(libelle_qualite("Near Mint"), "NM (Near Mint)");
/// assert_eq!(libelle_qualite("PL"), "PL (Played)");
/// assert_eq!(libelle_qualite("bizarre"), "bizarre");
/// assert_eq!(libelle_qualite(""), "");
/// ```
#[must_use]
pub fn libelle_qualite(ecriture: &str) -> String {
    match etat(ecriture) {
        Some(e) => format!("{} ({})", e.code, e.nom),
        None => ecriture.to_owned(),
    }
}

/// La phrase par laquelle Scanflip définit l'état, verbatim.
///
/// ```
/// use ygo_core::rarity::scanflip::explication_qualite;
/// assert!(explication_qualite("NM").unwrap().starts_with("Une carte Near Mint est très proche du neuf"));
/// assert!(explication_qualite("Damaged").is_some(), "toute écriture");
/// assert_eq!(explication_qualite("parfait"), None);
/// ```
#[must_use]
pub fn explication_qualite(ecriture: &str) -> Option<&'static str> {
    etat(ecriture).map(Etat::resume)
}

/// La section entière du guide pour cet état — pour les infobulles où l'on
/// choisit, et où les critères comptent plus que la phrase.
#[must_use]
pub fn detail_qualite(ecriture: &str) -> Option<String> {
    etat(ecriture).map(Etat::detail)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Les quarante-quatre raretés de la page d'aide sont là.
    #[test]
    fn la_table_est_complete() {
        assert_eq!(table().len(), 44);
        let codes: std::collections::HashSet<&str> =
            table().iter().map(|r| r.code.as_str()).collect();
        assert_eq!(codes.len(), 44, "aucun code en double");
        for attendu in ["C", "R", "SR", "U", "SCR", "COL", "QCR", "STR", "STRB"] {
            assert!(codes.contains(attendu), "{attendu} manque");
        }
    }

    /// Les deux écarts avec la table du Python, dans les deux sens.
    ///
    /// Ce test est la garde qui empêche de « corriger » l'un des deux
    /// fichiers pour les faire coïncider : ils décrivent deux choses
    /// différentes, et leur écart est une information.
    #[test]
    fn les_deux_ecarts_avec_la_table_du_python_sont_figes() {
        let python: std::collections::HashSet<&str> = super::super::reference::table()
            .iter()
            .map(|r| r.code.as_str())
            .collect();
        let scanflip: std::collections::HashSet<&str> =
            table().iter().map(|r| r.code.as_str()).collect();

        let seulement_scanflip: Vec<&&str> = scanflip.difference(&python).collect();
        assert_eq!(
            seulement_scanflip,
            vec![&"STRB"],
            "Starlight Blasonnée manque à la table du Python"
        );
        let seulement_python: Vec<&&str> = python.difference(&scanflip).collect();
        assert_eq!(
            seulement_python,
            vec![&"GMR"],
            "Grand Master Rare est une rareté OCG que Scanflip ignore"
        );
    }

    /// Un code que Scanflip refuse est reconnu comme tel.
    #[test]
    fn un_code_hors_regles_est_signale() {
        assert!(code_accepte("SCR"));
        assert!(code_accepte("scr"), "la casse n'entre pas en compte");
        assert!(code_accepte(" QCR "));
        assert!(!code_accepte("GMR"));
        assert!(!code_accepte(""));
        assert!(!code_accepte("Rareté Martienne"));
    }

    /// Toutes les formes que Scanflip annonce accepter, il faut les lire.
    #[test]
    fn les_ecritures_tolerees_sont_toutes_reconnues() {
        // Le code lui-même.
        assert_eq!(code_de("QCR"), Some("QCR"));
        // Le libellé complet, accents et casse indifférents.
        assert_eq!(code_de("Secrète Rare Quart de Siècle"), Some("QCR"));
        assert_eq!(code_de("SECRETE RARE QUART DE SIECLE"), Some("QCR"));
        // Le libellé amputé de son « Rare » final.
        for (court, attendu) in [
            ("Ultra", "U"),
            ("Super", "SR"),
            ("Secrète", "SCR"),
            ("Starlight", "STR"),
            ("Ultimate", "UTR"),
        ] {
            assert_eq!(code_de(court), Some(attendu), "« {court} »");
        }
        // L'apostrophe et le tiret ne comptent pas non plus.
        assert_eq!(code_de("Collectors Rare"), Some("COL"));
        assert_eq!(code_de(""), None);
        assert_eq!(code_de("  "), None);
    }

    /// « Rare » tout court reste la rareté « Rare », et non une rareté
    /// tronquée au hasard.
    #[test]
    fn rare_tout_court_reste_rare() {
        assert_eq!(code_de("Rare"), Some("R"));
        assert_eq!(code_de("R"), Some("R"));
    }

    /// Les qualités, dans toutes leurs écritures.
    #[test]
    fn les_qualites_se_ramenent_a_leur_code_court() {
        for (ecriture, attendu) in [
            ("M", "M"),
            ("Mint", "M"),
            ("NM", "NM"),
            ("NM+", "NM"),
            ("Near Mint", "NM"),
            ("near mint", "NM"),
            ("EX", "EX"),
            ("Excellent", "EX"),
            ("Lightly Played", "PL"),
            ("Damaged", "DM"),
        ] {
            assert_eq!(qualite_de(ecriture), Some(attendu), "« {ecriture} »");
        }
        assert_eq!(qualite_de("impeccable"), None);
        assert_eq!(qualite_de(""), None);
    }

    /// Le guide et la page d'aide de l'import désignent les **mêmes** sept
    /// états, dans le même ordre. S'ils divergeaient un jour — un état ajouté
    /// d'un côté seulement — ce test le dirait avant l'utilisateur.
    #[test]
    fn le_guide_et_l_import_connaissent_les_memes_etats() {
        let guide: Vec<&str> = guide_etats().iter().map(|e| e.code.as_str()).collect();
        let import: Vec<&str> = index()
            .regles
            .qualites
            .iter()
            .filter_map(|l| l.first().map(String::as_str))
            .collect();
        assert_eq!(guide, import);
        assert_eq!(guide, vec!["M", "NM", "EX", "GD", "PL", "PO", "DM"]);
    }

    /// La recopie est complète : chaque état a sa phrase et au moins un
    /// critère, et les passages cités ailleurs dans le projet y sont.
    #[test]
    fn la_recopie_du_guide_est_complete() {
        for e in guide_etats() {
            assert!(!e.phrase.is_empty(), "{} sans phrase", e.code);
            assert!(!e.criteres.is_empty(), "{} sans critère", e.code);
            assert_ne!(e.nom, e.code);
        }
        let mint = etat("M").unwrap();
        assert_eq!(mint.toleres.len(), 2, "les deux défauts tolérés");
        assert!(mint.remarques[0].contains("note de 9 ou 10"));
        assert!(etat("DM").unwrap().remarques[0].contains("automatiquement classée"));
        assert_eq!(
            etat("EX").unwrap().resume(),
            "Une carte en état 'Excellent' reste une très belle carte qui peut présenter \
             des défauts visibles à l'inspection rapide",
            "les deux-points qui annoncent la liste sont retirés du résumé"
        );
    }

    /// L'écart réel entre les deux pages de Scanflip, figé : le guide dit
    /// « Played », l'import tolère « Lightly Played ». Les deux mènent à PL.
    #[test]
    fn played_et_lightly_played_sont_le_meme_etat() {
        assert_eq!(nom_qualite("PL"), Some("Played"));
        assert_eq!(etat("Played").unwrap().code, "PL");
        assert_eq!(etat("Lightly Played").unwrap().code, "PL");
        assert_eq!(qualite_de("Lightly Played"), Some("PL"));
    }

    /// Le détail d'une infobulle reprend toute la section.
    #[test]
    fn le_detail_reprend_toute_la_section() {
        let d = detail_qualite("M").unwrap();
        assert!(d.starts_with("M — Mint\n"));
        assert!(d.contains("  • Coins nets et bien formés"));
        assert!(d.contains("Tolérés :"));
        assert!(d.contains("note de 9 ou 10"));
        assert!(
            !detail_qualite("PL").unwrap().contains("Tolérés"),
            "pas de rubrique vide"
        );
    }

    /// `NM+` est une écriture de Scanflip que notre échelle ne connaît pas :
    /// elle doit se ramener à `NM`, sans quoi un import la stockerait telle
    /// quelle et le tri par état verrait deux valeurs pour un même état.
    #[test]
    fn nm_plus_se_ramene_a_nm() {
        assert_eq!(qualite_de("NM+"), Some("NM"));
        assert_ne!(qualite_de("NM+"), Some("NM+"));
    }
}
