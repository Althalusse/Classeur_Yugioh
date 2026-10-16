// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le numéro de version **de l'application** — `2.0.0-alpha.2`.
//!
//! À ne pas confondre avec [`crate::version`], qui suit la version de la
//! **base de référence** (`147.20`).
//!
//! # Pourquoi un comparateur à nous
//!
//! La mise à jour de l'application compare le numéro de l'exécutable à celui
//! des Releases GitHub. Comparer des chaînes ferait passer `2.0.0-alpha.10`
//! avant `2.0.0-alpha.9`, et `2.0.0` avant `2.0.0-alpha.1`. La règle est celle
//! de *Semantic Versioning* (§ 11) :
//!
//! - majeur, mineur, correctif comparés comme des nombres ;
//! - une version **avec** suffixe (`-alpha.1`) précède la même **sans**
//!   suffixe : une pré-version vient avant la version finale ;
//! - les suffixes se comparent identifiant par identifiant : nombres entre
//!   eux, textes entre eux, un nombre avant un texte, le plus court d'abord à
//!   égalité.
//!
//! Les tags du dépôt n'ont pas toujours été écrits pareil (`V1.0.3`, `1.0.1`,
//! `1.0`) : le `v` initial est ignoré, un correctif ou un mineur absent vaut
//! `0`. Ce qui suit un `+` (métadonnées de build) ne compte pas.

use std::cmp::Ordering;

/// Un numéro de version comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumeroVersion {
    majeur: u64,
    mineur: u64,
    correctif: u64,
    pre: Vec<Identifiant>,
}

/// Un identifiant de pré-version : `alpha`, `2`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identifiant {
    Nombre(u64),
    Texte(String),
}

impl Ord for Identifiant {
    fn cmp(&self, autre: &Self) -> Ordering {
        match (self, autre) {
            (Self::Nombre(a), Self::Nombre(b)) => a.cmp(b),
            (Self::Texte(a), Self::Texte(b)) => a.cmp(b),
            (Self::Nombre(_), Self::Texte(_)) => Ordering::Less,
            (Self::Texte(_), Self::Nombre(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Identifiant {
    fn partial_cmp(&self, autre: &Self) -> Option<Ordering> {
        Some(self.cmp(autre))
    }
}

impl NumeroVersion {
    /// Lit un numéro — `v2.0.0-alpha.2`, `2.0.0`, `V1.0.3`, `1.0`.
    ///
    /// Rend `None` pour ce qui n'est pas un numéro : un tag de Release mal
    /// formé n'est jamais proposé comme mise à jour.
    ///
    /// ```
    /// use ygo_core::numero_version::NumeroVersion;
    /// let lire = |t| NumeroVersion::lire(t).unwrap();
    /// assert!(lire("v2.0.0-alpha.2") > lire("2.0.0-alpha.1"));
    /// assert!(lire("2.0.0-alpha.10") > lire("2.0.0-alpha.9"));
    /// assert!(lire("2.0.0") > lire("2.0.0-alpha.10"));
    /// assert!(lire("2.0.0-alpha.1") > lire("V1.0.3"));
    /// assert_eq!(NumeroVersion::lire("Ajout Inventaire"), None);
    /// ```
    #[must_use]
    pub fn lire(texte: &str) -> Option<Self> {
        let texte = texte.trim();
        let texte = texte
            .strip_prefix('v')
            .or_else(|| texte.strip_prefix('V'))
            .unwrap_or(texte);
        let texte = texte.split('+').next().unwrap_or(texte);
        let (noyau, pre) = match texte.split_once('-') {
            Some((n, p)) => (n, Some(p)),
            None => (texte, None),
        };
        let mut parties = noyau.split('.');
        let nombre = |p: Option<&str>| -> Option<u64> {
            match p {
                None => Some(0),
                Some(p) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => p.parse().ok(),
                Some(_) => None,
            }
        };
        let majeur = match parties.next() {
            Some(p) if !p.is_empty() => nombre(Some(p))?,
            _ => return None,
        };
        let mineur = nombre(parties.next())?;
        let correctif = nombre(parties.next())?;
        if parties.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => {
                let mut ids = Vec::new();
                for id in p.split('.') {
                    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                        return None;
                    }
                    ids.push(if id.chars().all(|c| c.is_ascii_digit()) {
                        Identifiant::Nombre(id.parse().ok()?)
                    } else {
                        Identifiant::Texte(id.to_ascii_lowercase())
                    });
                }
                ids
            }
        };
        Some(Self {
            majeur,
            mineur,
            correctif,
            pre,
        })
    }

    /// Est-ce une pré-version (`-alpha`, `-beta`, `-rc`…) ?
    #[must_use]
    pub fn est_pre_version(&self) -> bool {
        !self.pre.is_empty()
    }
}

/// La forme normalisée : `2.0.1`, `2.0.0-alpha.2` — sans `v`, mineur et
/// correctif toujours écrits.
impl std::fmt::Display for NumeroVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.majeur, self.mineur, self.correctif)?;
        for (i, id) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match id {
                Identifiant::Nombre(n) => write!(f, "{n}")?,
                Identifiant::Texte(t) => f.write_str(t)?,
            }
        }
        Ok(())
    }
}

impl Ord for NumeroVersion {
    fn cmp(&self, autre: &Self) -> Ordering {
        (self.majeur, self.mineur, self.correctif)
            .cmp(&(autre.majeur, autre.mineur, autre.correctif))
            .then_with(|| match (self.pre.is_empty(), autre.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // Sans suffixe : la version finale, après toutes ses
                // pré-versions.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&autre.pre),
            })
    }
}

impl PartialOrd for NumeroVersion {
    fn partial_cmp(&self, autre: &Self) -> Option<Ordering> {
        Some(self.cmp(autre))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn v(t: &str) -> NumeroVersion {
        NumeroVersion::lire(t).unwrap()
    }

    /// L'ordre de la spécification SemVer, exemple du § 11.4.
    #[test]
    fn l_ordre_est_celui_de_semver() {
        let ordre = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "2.0.0-alpha.1",
            "2.0.0-alpha.2",
            "2.0.0",
            "2.1.0",
            "10.0.0",
        ];
        for paire in ordre.windows(2) {
            if let [a, b] = paire {
                assert!(v(a) < v(b), "{a} < {b}");
            }
        }
    }

    /// Les tags réels du dépôt, écrits de trois façons.
    #[test]
    fn les_anciens_tags_du_depot_se_lisent() {
        assert_eq!(v("V1.0.3"), v("1.0.3"));
        assert_eq!(v("1.0"), v("1.0.0"));
        assert!(v("1.0.1") < v("V1.0.3"));
        assert!(v("V1.0.3") < v("v2.0.0-alpha.1"));
        assert!(v("2.0.0-alpha.1").est_pre_version());
        assert!(!v("V1.0.3").est_pre_version());
    }

    #[test]
    fn ce_qui_n_est_pas_un_numero_est_refuse() {
        for t in [
            "",
            "v",
            "Ajout Inventaire",
            "1.x.0",
            "1.0.0.0",
            "1.0.0-",
            "1.0.0-a..b",
        ] {
            assert_eq!(NumeroVersion::lire(t), None, "{t:?}");
        }
    }

    #[test]
    fn la_forme_affichee_est_normalisee() {
        assert_eq!(v("V1.0").to_string(), "1.0.0");
        assert_eq!(v("v2.0.0-alpha.2").to_string(), "2.0.0-alpha.2");
    }

    #[test]
    fn les_metadonnees_de_build_ne_comptent_pas() {
        assert_eq!(v("2.0.0+windows"), v("2.0.0"));
    }
}
