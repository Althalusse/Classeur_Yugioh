// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les exemplaires d'une carte, à l'écran.
//!
//! Les règles vivent dans [`ygo_app::exemplaires`] : qui part quand la
//! quantité baisse, quand un exemplaire redevient standard, ce que « pour
//! tous » écrit. Ici, il ne reste que des listes déroulantes, qui rendent
//! des [`Demande`] — jamais une écriture.
//!
//! Deux écrans s'en servent :
//!
//! - l'**inventaire**, où une carte de plusieurs exemplaires se déplie en
//!   lignes, une par exemplaire ;
//! - le **classeur**, où le clic droit ▸ « Qualité… » ouvre [`Fenetre`].

use std::hash::Hash;

use eframe::egui;
use ygo_app::exemplaires::{Champ, Demande, Etat, Exemplaire, Exemplaires};
use ygo_app::scanflip::{self, Edition};
use ygo_core::rarity::scanflip::{detail_qualite, libelle_qualite};

/// Les éditions proposées, dans l'ordre des colonnes de Scanflip.
pub const EDITIONS: [Edition; 3] = [Edition::Premiere, Edition::Illimitee, Edition::Limitee];

/// Le libellé d'une édition stockée — tiret quand elle n'est pas renseignée.
///
/// ```
/// use ygo_ui::exemplaires::libelle_edition;
/// assert_eq!(libelle_edition("1st"), "1st Edition");
/// assert_eq!(libelle_edition("unlimited"), "Unlimited");
/// assert_eq!(libelle_edition(""), "—");
/// ```
#[must_use]
pub fn libelle_edition(code: &str) -> String {
    if code.trim().is_empty() {
        return "—".to_owned();
    }
    Edition::depuis_code(code).map_or_else(|| code.to_owned(), |e| e.colonne().to_owned())
}

/// L'état d'une carte en un mot : le code quand tous ses exemplaires
/// l'ont, le décompte sinon.
///
/// ```
/// use ygo_app::exemplaires::{Etat, Exemplaires};
/// use ygo_ui::exemplaires::resume;
/// let nm = Etat::nouveau("NM", "1st");
/// let ex = Exemplaires::depuis(5, nm.clone(), vec![]);
/// assert_eq!(resume(&ex), "NM");
/// let ex = Exemplaires::depuis(5, nm, vec![(1, Etat::nouveau("PL", "1st")), (2, Etat::nouveau("PL", "1st"))]);
/// assert_eq!(resume(&ex), "NM ×3 · PL ×2");
/// // L'édition seule diffère : le code se répète, le décompte le dit.
/// let ex = Exemplaires::depuis(2, Etat::nouveau("NM", "1st"), vec![(1, Etat::nouveau("NM", "unlimited"))]);
/// assert_eq!(resume(&ex), "NM ×1 · NM ×1");
/// ```
#[must_use]
pub fn resume(ex: &Exemplaires) -> String {
    let code = |q: &str| {
        if q.is_empty() {
            "—".to_owned()
        } else {
            q.to_owned()
        }
    };
    if ex.homogene() {
        return ex
            .groupes()
            .first()
            .map_or_else(|| code(&ex.commun.qualite), |(e, _)| code(&e.qualite));
    }
    ex.groupes()
        .iter()
        .map(|(e, n)| format!("{} ×{n}", code(&e.qualite)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// La liste déroulante d'un état. Rend `true` si l'utilisateur a choisi.
pub fn choix_etat(ui: &mut egui::Ui, id: impl Hash + std::fmt::Debug, valeur: &mut String) -> bool {
    let avant = valeur.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(if valeur.is_empty() {
            "—".to_owned()
        } else {
            libelle_qualite(valeur)
        })
        .width(150.0)
        .show_ui(ui, |ui| {
            for e in scanflip::ETATS {
                let entree = ui.selectable_value(valeur, e.to_owned(), libelle_qualite(e));
                if let Some(texte) = detail_qualite(e) {
                    entree.on_hover_text(texte);
                }
            }
            ui.separator();
            ui.selectable_value(valeur, String::new(), "(non renseigné)");
        });
    *valeur != avant
}

/// La liste déroulante d'une édition. Rend `true` si l'utilisateur a choisi.
pub fn choix_edition(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug,
    valeur: &mut String,
) -> bool {
    let avant = valeur.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(libelle_edition(valeur))
        .width(130.0)
        .show_ui(ui, |ui| {
            for e in EDITIONS {
                ui.selectable_value(valeur, e.code().to_owned(), e.colonne());
            }
            ui.separator();
            ui.selectable_value(valeur, String::new(), "(non renseignée)");
        });
    *valeur != avant
}

/// Les deux listes et le bouton d'un exemplaire, côte à côte.
///
/// Rend la demande, s'il y en a une.
pub fn reglages(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug + Clone,
    exemplaire: Exemplaire,
    etat: &Etat,
) -> Option<Demande> {
    let mut demande = None;
    let mut qualite = etat.qualite.clone();
    if choix_etat(ui, (id.clone(), "etat"), &mut qualite) {
        demande = Some(Demande::Modifier(
            exemplaire,
            Etat::nouveau(&qualite, &etat.edition),
        ));
    }
    let mut edition = etat.edition.clone();
    if choix_edition(ui, (id, "edition"), &mut edition) {
        demande = Some(Demande::Modifier(
            exemplaire,
            Etat::nouveau(&etat.qualite, &edition),
        ));
    }
    if ui
        .small_button("Retirer")
        .on_hover_text("Retirer cet exemplaire : la quantité baisse d'un")
        .clicked()
    {
        demande = Some(Demande::Retirer(exemplaire));
    }
    demande
}

/// La valeur que montre une liste « pour tous » quand les exemplaires ne
/// s'accordent pas : aucune entrée ne l'a, elle ne peut donc pas être
/// choisie par erreur.
const MIXTE: &str = "\u{0}mixte";

/// La fenêtre « Qualité… » du classeur : une carte, ses exemplaires.
#[derive(Debug, Clone)]
pub struct Fenetre {
    /// La ligne du classeur.
    pub rowid: i64,
    /// Ce qui la nomme : nom, code, rareté.
    pub titre: String,
    /// Les exemplaires tels que la base les a rendus.
    pub exemplaires: Exemplaires,
    /// La dernière écriture refusée, s'il y en a une.
    pub erreur: Option<String>,
}

impl Fenetre {
    /// Dessine la fenêtre. Rend `(ouverte, demande)`.
    pub fn montrer(&mut self, ctx: &egui::Context) -> (bool, Option<Demande>) {
        let mut ouverte = true;
        let mut demande = None;
        egui::Window::new("Qualité des exemplaires")
            .id(egui::Id::new(("qualite-exemplaires", self.rowid)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.label(egui::RichText::new(&self.titre).strong());
                ui.small(format!(
                    "{} exemplaire(s) — {}",
                    self.exemplaires.quantite,
                    resume(&self.exemplaires)
                ));
                ui.add_space(8.0);

                if self.exemplaires.quantite > 1 {
                    if let Some(d) = self.pour_tous(ui) {
                        demande = Some(d);
                    }
                    ui.separator();
                }

                let liste = self.exemplaires.liste();
                if liste.is_empty() {
                    ui.weak("Aucun exemplaire possédé.");
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (k, (exemplaire, etat)) in liste.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let a_part = matches!(exemplaire, Exemplaire::APart(_));
                                let libelle = format!("Exemplaire {}", k + 1);
                                ui.add_sized(
                                    [96.0, 20.0],
                                    egui::Label::new(if a_part {
                                        egui::RichText::new(libelle).strong()
                                    } else {
                                        egui::RichText::new(libelle)
                                    }),
                                )
                                .on_hover_text(if a_part {
                                    "Réglé à part"
                                } else {
                                    "Standard : il suit les réglages « pour tous »"
                                });
                                if let Some(d) =
                                    reglages(ui, ("fenetre", self.rowid, k), *exemplaire, etat)
                                {
                                    demande = Some(d);
                                }
                            });
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Ajouter un exemplaire").clicked() {
                        demande = Some(Demande::Ajouter);
                    }
                });
                if let Some(e) = &self.erreur {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            });
        (ouverte, demande)
    }

    /// La ligne « Tous les exemplaires » : écrit un champ sur chacun.
    fn pour_tous(&self, ui: &mut egui::Ui) -> Option<Demande> {
        let mut demande = None;
        let groupes = self.exemplaires.groupes();
        let commun = |champ: fn(&Etat) -> &String| -> String {
            let premier = groupes.first().map(|(e, _)| champ(e).clone());
            match premier {
                Some(p) if groupes.iter().all(|(e, _)| *champ(e) == p) => p,
                _ => MIXTE.to_owned(),
            }
        };
        ui.horizontal(|ui| {
            ui.add_sized(
                [96.0, 20.0],
                egui::Label::new(egui::RichText::new("Tous").strong()),
            )
            .on_hover_text("Écrit l'état ou l'édition sur chaque exemplaire");
            let mut qualite = commun(|e| &e.qualite);
            let avant = qualite.clone();
            egui::ComboBox::from_id_salt(("tous-etat", self.rowid))
                .selected_text(if qualite == MIXTE {
                    "(mixte)".to_owned()
                } else if qualite.is_empty() {
                    "—".to_owned()
                } else {
                    libelle_qualite(&qualite)
                })
                .width(150.0)
                .show_ui(ui, |ui| {
                    for e in scanflip::ETATS {
                        ui.selectable_value(&mut qualite, e.to_owned(), libelle_qualite(e));
                    }
                    ui.separator();
                    ui.selectable_value(&mut qualite, String::new(), "(non renseigné)");
                });
            if qualite != avant {
                demande = Some(Demande::Tous(Champ::Qualite, qualite));
            }
            let mut edition = commun(|e| &e.edition);
            let avant = edition.clone();
            egui::ComboBox::from_id_salt(("tous-edition", self.rowid))
                .selected_text(if edition == MIXTE {
                    "(mixte)".to_owned()
                } else {
                    libelle_edition(&edition)
                })
                .width(130.0)
                .show_ui(ui, |ui| {
                    for e in EDITIONS {
                        ui.selectable_value(&mut edition, e.code().to_owned(), e.colonne());
                    }
                    ui.separator();
                    ui.selectable_value(&mut edition, String::new(), "(non renseignée)");
                });
            if edition != avant {
                demande = Some(Demande::Tous(Champ::Edition, edition));
            }
        });
        demande
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn un_etat_vide_se_resume_en_tiret() {
        let ex = Exemplaires::depuis(3, Etat::default(), vec![]);
        assert_eq!(resume(&ex), "—");
        let ex = Exemplaires::depuis(2, Etat::default(), vec![(1, Etat::nouveau("PL", ""))]);
        assert_eq!(resume(&ex), "— ×1 · PL ×1");
    }

    /// La valeur « mixte » n'est aucune des valeurs qu'on peut choisir.
    #[test]
    fn la_valeur_mixte_ne_se_confond_avec_rien() {
        assert!(!scanflip::ETATS.contains(&MIXTE));
        assert!(EDITIONS.iter().all(|e| e.code() != MIXTE));
        assert!(!MIXTE.is_empty());
    }

    /// La fenêtre se dessine sans rien demander tant qu'on n'y touche pas.
    #[test]
    fn la_fenetre_ne_demande_rien_d_elle_meme() {
        let ctx = egui::Context::default();
        let mut f = Fenetre {
            rowid: 1,
            titre: "Dark Magician — RA02-EN001 — Ultra Rare".into(),
            exemplaires: Exemplaires::depuis(
                3,
                Etat::nouveau("NM", "1st"),
                vec![(4, Etat::nouveau("PL", "1st"))],
            ),
            erreur: None,
        };
        let mut rendu = (false, None);
        let mut sortie = ctx.run_ui(egui::RawInput::default(), |ui| {
            rendu = f.montrer(ui.ctx());
        });
        sortie.textures_delta.clear();
        assert!(rendu.0, "ouverte");
        assert_eq!(rendu.1, None);
    }
}
