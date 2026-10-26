// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les raretés qui portent un nom par région.
//!
//! # Le cas — 2026-10-26
//!
//! La Ghost Rare du TCG s'appelle **Holographic Rare** en OCG
//! (ホログラフィックレア) : Yugipedia la décrit comme « la version OCG de la
//! Ghost Rare ». Les sources ne s'accordent pas sur le nom :
//!
//! | Source | `INFO-JP033` « The Unstoppable Exodia Incarnate » |
//! |---|---|
//! | YGOJSON (`cardinfo.db`) | `Ghost Rare` — son code `ghost` vaut pour toutes les régions |
//! | Set list Yugipedia (OCG-JP) | `Holographic Rare` |
//! | fichier Yugipedia | `TheUnstoppableExodiaIncarnate-INFO-JP-HGR.png` |
//! | Cardmarket (extension OCG) | « Holographic Rare » |
//!
//! Le classeur `INFO-JP` recevait donc de la base une ligne `Ghost Rare` ; la
//! passe artworks, lisant `Holographic Rare` dans la Set list sans la
//! retrouver, en **insérait une seconde** — la même carte — sous un libellé
//! que le référentiel ignorait : « libellé inconnu du référentiel ». Toutes
//! les Holographic de la base sont dans ce cas : 92 tirages japonais, un par
//! booster OCG depuis *The Duelist Genesis*.
//!
//! # Deux noms, une rareté
//!
//! - [`nom_de_reference`] ramène un nom régional au nom TCG : c'est lui que
//!   comparent la clé de rareté ([`super::canon::cle`]), les règles
//!   Cardmarket et, à défaut d'une priorité propre, les priorités
//!   ([`super::Priorites::pour_tri`]) — `Holographic Rare` et `Ghost Rare` y
//!   sont **la même** rareté ;
//! - [`nom_pour_le_numero`] donne le nom à **écrire** dans le classeur selon
//!   la région du numéro : `Holographic Rare` pour `INFO-JP033`, `Ghost Rare`
//!   pour un numéro TCG — celui qu'on lit sur la carte qu'on a en main.

use crate::config::SUFFIXES_OCG;

/// Les raretés à double nom : `(nom OCG, nom TCG)`.
pub const NOMS_REGIONAUX: [(&str, &str); 1] = [("Holographic Rare", "Ghost Rare")];

/// Le nom TCG d'une rareté — le libellé lui-même s'il n'a pas de nom
/// régional.
///
/// ```
/// use ygo_core::rarity::region::nom_de_reference;
/// assert_eq!(nom_de_reference("Holographic Rare"), "Ghost Rare");
/// assert_eq!(nom_de_reference("Ghost Rare"), "Ghost Rare");
/// assert_eq!(nom_de_reference("Ultra Rare"), "Ultra Rare");
/// ```
#[must_use]
pub fn nom_de_reference(libelle: &str) -> &str {
    NOMS_REGIONAUX
        .iter()
        .find(|(ocg, _)| *ocg == libelle)
        .map_or(libelle, |(_, tcg)| tcg)
}

/// L'autre nom d'une rareté à double nom, dans un sens comme dans l'autre.
///
/// ```
/// use ygo_core::rarity::region::jumelle;
/// assert_eq!(jumelle("Holographic Rare"), Some("Ghost Rare"));
/// assert_eq!(jumelle("Ghost Rare"), Some("Holographic Rare"));
/// assert_eq!(jumelle("Secret Rare"), None);
/// ```
#[must_use]
pub fn jumelle(libelle: &str) -> Option<&'static str> {
    NOMS_REGIONAUX.iter().find_map(|(ocg, tcg)| {
        if *ocg == libelle {
            Some(*tcg)
        } else if *tcg == libelle {
            Some(*ocg)
        } else {
            None
        }
    })
}

/// La langue d'un numéro : les deux lettres qui suivent le premier tiret
/// (`INFO-JP033` → `JP`, `CORI-JPS10` → `JP`, `LDK2-ENK01` → `EN`). `None`
/// pour un numéro sans langue (`LOB-001`).
fn langue_du_numero(set_code: &str) -> Option<String> {
    let (_, reste) = set_code.trim().split_once('-')?;
    let langue: String = reste.chars().take(2).collect();
    (langue.chars().count() == 2 && langue.chars().all(|c| c.is_ascii_alphabetic()))
        .then(|| langue.to_ascii_uppercase())
}

/// Le numéro est-il imprimé pour l'OCG — japonais, coréen, anglais asiatique,
/// chinois ([`SUFFIXES_OCG`]) ?
///
/// ```
/// use ygo_core::rarity::region::numero_ocg;
/// assert!(numero_ocg("INFO-JP033"));
/// assert!(numero_ocg("CORI-JPS10"), "la lettre du sous-jeu ne gêne pas");
/// assert!(numero_ocg("INFO-AE033"));
/// assert!(!numero_ocg("INFO-FR033"));
/// assert!(!numero_ocg("LDK2-ENK01"));
/// assert!(!numero_ocg("LOB-001"));
/// ```
#[must_use]
pub fn numero_ocg(set_code: &str) -> bool {
    langue_du_numero(set_code).is_some_and(|l| SUFFIXES_OCG.contains(&l.as_str()))
}

/// Le nom que la rareté doit porter sur ce numéro, quand ce n'est **pas**
/// `libelle` — `None` s'il n'y a rien à changer.
///
/// Un numéro sans langue lisible ne change rien : sans région, pas de nom
/// régional.
///
/// ```
/// use ygo_core::rarity::region::nom_pour_le_numero;
/// assert_eq!(nom_pour_le_numero("Ghost Rare", "INFO-JP033"), Some("Holographic Rare"));
/// assert_eq!(nom_pour_le_numero("Holographic Rare", "INFO-JP033"), None);
/// assert_eq!(nom_pour_le_numero("Holographic Rare", "LODT-EN000"), Some("Ghost Rare"));
/// assert_eq!(nom_pour_le_numero("Ghost Rare", "LODT-EN000"), None);
/// assert_eq!(nom_pour_le_numero("Ultra Rare", "INFO-JP033"), None);
/// assert_eq!(nom_pour_le_numero("Holographic Rare", "LOB-001"), None);
/// ```
#[must_use]
pub fn nom_pour_le_numero(libelle: &str, set_code: &str) -> Option<&'static str> {
    let langue = langue_du_numero(set_code)?;
    let ocg = SUFFIXES_OCG.contains(&langue.as_str());
    NOMS_REGIONAUX.iter().find_map(|(nom_ocg, nom_tcg)| {
        if ocg && libelle == *nom_tcg {
            Some(*nom_ocg)
        } else if !ocg && libelle == *nom_ocg {
            Some(*nom_tcg)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Chaque nom régional est connu du référentiel, et sous son propre
    /// code : sans quoi la canonisation le déclarerait inconnu — le défaut
    /// du 2026-10-26.
    #[test]
    fn chaque_nom_regional_est_au_referentiel() {
        for (ocg, tcg) in NOMS_REGIONAUX {
            let code_ocg = crate::rarity::reference::nom_vers_code(ocg).unwrap();
            let code_tcg = crate::rarity::reference::nom_vers_code(tcg).unwrap();
            assert_ne!(code_ocg, code_tcg, "{ocg} / {tcg}");
            assert_eq!(crate::rarity::reference::code_vers_nom_en(code_ocg), ocg);
        }
    }

    /// Les quatre régions OCG du jeu, et leurs numéros réels.
    #[test]
    fn les_numeros_ocg_prennent_le_nom_ocg() {
        for numero in [
            "INFO-JP033",
            "INFO-KR033",
            "INFO-AE033",
            "INFO-SC033",
            "DUEA-JA000",
        ] {
            assert_eq!(
                nom_pour_le_numero("Ghost Rare", numero),
                Some("Holographic Rare"),
                "{numero}"
            );
        }
        for numero in ["INFO-EN033", "LODT-FR000", "RA05-DE001", "SDWD-EN013"] {
            assert_eq!(nom_pour_le_numero("Ghost Rare", numero), None, "{numero}");
        }
    }

    #[test]
    fn un_numero_sans_langue_ne_change_rien() {
        for numero in ["", "LOB", "LOB-001", "SDK-E001", "-"] {
            assert_eq!(nom_pour_le_numero("Ghost Rare", numero), None, "{numero:?}");
            assert_eq!(
                nom_pour_le_numero("Holographic Rare", numero),
                None,
                "{numero:?}"
            );
        }
    }
}
