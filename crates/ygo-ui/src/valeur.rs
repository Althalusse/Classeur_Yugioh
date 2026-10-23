// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La valeur des cartes possédées, selon les prix Cardmarket : la pastille
//! des tuiles de l'accueil, le montant des barres de l'accueil et du
//! classeur.
//!
//! Le chiffrage lit l'offre de chaque ligne (`ygo_app::cardmarket`) : un
//! dixième de seconde pour 1 300 lignes possédées, plusieurs secondes sur un
//! disque froid. Il se fait donc sur un fil ([`Calcul`]), jamais dans le
//! rendu. Les textes sont des fonctions pures.
//!
//! Ne sont chiffrés que les exemplaires dont la version est sûre — ceux dont
//! la fiche ouvre la page même du produit. Les autres sont comptés à part et
//! l'info-bulle le dit : un montant qui inclurait la fourchette d'une carte
//! dont on ignore la version chiffrerait peut-être une Starlight au prix
//! d'une Super.

use std::sync::mpsc::{Receiver, TryRecvError};

use ygo_app::cardmarket::{euros, euros_ronds, Valeur};

/// Un calcul sur un fil, dont le résultat se relève à chaque image.
pub struct Calcul<T> {
    reception: Option<Receiver<T>>,
}

impl<T> std::fmt::Debug for Calcul<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Calcul")
            .field("en_cours", &self.reception.is_some())
            .finish()
    }
}

impl<T> Default for Calcul<T> {
    fn default() -> Self {
        Self { reception: None }
    }
}

impl<T: Send + 'static> Calcul<T> {
    /// Lance `travail` sur un fil nommé `nom`. Un fil qui ne démarre pas
    /// laisse un calcul vide : l'écran s'en passe.
    pub fn lancer(nom: &str, travail: impl FnOnce() -> T + Send + 'static) -> Self {
        let (envoi, reception) = std::sync::mpsc::channel();
        let fil = std::thread::Builder::new()
            .name(nom.to_owned())
            .spawn(move || {
                let _ = envoi.send(travail());
            });
        match fil {
            Ok(_) => Self {
                reception: Some(reception),
            },
            Err(e) => {
                tracing::error!(erreur = %e, fil = nom, "calcul non démarré");
                Self::default()
            }
        }
    }
}

impl<T> Calcul<T> {
    /// Le calcul tourne-t-il encore ?
    #[must_use]
    pub fn en_cours(&self) -> bool {
        self.reception.is_some()
    }

    /// Le résultat, s'il vient d'arriver — une seule fois. Un fil mort sans
    /// rien rendre clôt le calcul.
    pub fn relever(&mut self) -> Option<T> {
        let resultat = match self.reception.as_ref()?.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        self.reception = None;
        resultat
    }
}

/// La pastille d'une tuile, en euros ronds — rien sans exemplaire chiffré.
///
/// ```
/// use ygo_app::cardmarket::Valeur;
/// use ygo_ui::valeur::pastille;
/// let v = Valeur { centimes: 15_240, chiffres: 12, non_chiffres: 3 };
/// assert_eq!(pastille(&v).unwrap(), "≈ 152 €");
/// assert_eq!(pastille(&Valeur { non_chiffres: 4, ..Valeur::default() }), None);
/// ```
#[must_use]
pub fn pastille(v: &Valeur) -> Option<String> {
    (v.chiffres > 0).then(|| format!("≈ {}", euros_ronds(v.centimes)))
}

/// Le montant d'une barre, au centime — rien sans exemplaire chiffré.
///
/// ```
/// use ygo_app::cardmarket::Valeur;
/// use ygo_ui::valeur::montant;
/// let v = Valeur { centimes: 160_018, chiffres: 1_242, non_chiffres: 109 };
/// assert_eq!(montant(&v).unwrap(), "≈ 1 600,18 €");
/// assert_eq!(montant(&Valeur::default()), None);
/// ```
#[must_use]
pub fn montant(v: &Valeur) -> Option<String> {
    (v.chiffres > 0).then(|| format!("≈ {}", euros(v.centimes)))
}

/// Le prix d'un exemplaire, au bas d'une carte du classeur : au centime
/// sous 100 €, en euros ronds au-delà — la place est comptée.
///
/// ```
/// use ygo_ui::valeur::prix_sur_la_carte;
/// assert_eq!(prix_sur_la_carte(388), "3,88 €");
/// assert_eq!(prix_sur_la_carte(9_999), "99,99 €");
/// assert_eq!(prix_sur_la_carte(38_696), "387 €");
/// assert_eq!(prix_sur_la_carte(123_456), "1 235 €");
/// ```
#[must_use]
pub fn prix_sur_la_carte(centimes: u32) -> String {
    if centimes < 10_000 {
        euros(centimes.into())
    } else {
        euros_ronds(centimes.into())
    }
}

/// Le détail, pour une info-bulle : ce qui est compté, ce qui ne l'est pas,
/// d'où viennent les prix. Vide sans exemplaire.
///
/// ```
/// use ygo_app::cardmarket::Valeur;
/// use ygo_ui::valeur::detail;
/// let v = Valeur { centimes: 15_240, chiffres: 12, non_chiffres: 3 };
/// let texte = detail(&v, "2026-10-20T02:44:58+0200");
/// assert!(texte.starts_with("Valeur estimée : 152,40 € pour 12 exemplaire(s)"));
/// assert!(texte.contains("3 exemplaire(s) sans prix sûr"));
/// assert!(texte.contains("20/10/2026"));
/// assert_eq!(detail(&Valeur::default(), ""), "");
/// ```
#[must_use]
pub fn detail(v: &Valeur, prix_du: &str) -> String {
    if v.exemplaires() == 0 {
        return String::new();
    }
    let mut lignes = Vec::new();
    if v.chiffres > 0 {
        lignes.push(format!(
            "Valeur estimée : {} pour {} exemplaire(s) possédé(s)",
            euros(v.centimes),
            v.chiffres
        ));
    }
    if v.non_chiffres > 0 {
        lignes.push(format!(
            "{} exemplaire(s) sans prix sûr, non compté(s) : version incertaine, ou carte \
             non retrouvée chez Cardmarket",
            v.non_chiffres
        ));
    }
    lignes.push(note(prix_du));
    lignes.join("\n")
}

/// D'où viennent les prix — la même portée que sur la fiche.
///
/// ```
/// use ygo_ui::valeur::note;
/// assert_eq!(
///     note("2026-10-20T02:44:58+0200"),
///     "Moyenne des ventes sur 30 jours × quantité · prix Cardmarket du 20/10/2026, \
///      toutes langues, éditions et états confondus"
/// );
/// ```
#[must_use]
pub fn note(prix_du: &str) -> String {
    let date = crate::fiche::date_courte(prix_du).map_or_else(String::new, |d| format!(" du {d}"));
    format!(
        "Moyenne des ventes sur 30 jours × quantité · prix Cardmarket{date}, toutes langues, \
         éditions et états confondus"
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn le_resultat_se_releve_une_fois() {
        let mut calcul = Calcul::lancer("test-valeur", || 42);
        let mut recu = None;
        // Jusqu'à dix secondes : la boucle sort dès le résultat, mais une
        // machine chargée peut tarder à ordonnancer le fil.
        for _ in 0..2_000 {
            if let Some(r) = calcul.relever() {
                recu = Some(r);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(recu, Some(42));
        assert!(!calcul.en_cours());
        assert_eq!(calcul.relever(), None);
    }

    #[test]
    fn un_fil_mort_clot_le_calcul() {
        let mut calcul: Calcul<u8> = Calcul::lancer("test-valeur-mort", || -> u8 {
            std::panic::panic_any("échec simulé")
        });
        // Jusqu'à dix secondes : avec `RUST_BACKTRACE`, la trace d'une
        // panique se résout avant que le fil meure, et elle a dépassé la
        // seconde sur une machine chargée (2026-10-23).
        for _ in 0..2_000 {
            if !calcul.en_cours() {
                break;
            }
            let _ = calcul.relever();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!calcul.en_cours());
    }
}
