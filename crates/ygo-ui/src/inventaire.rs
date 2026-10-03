// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran de l'inventaire : toutes les cartes possédées, et ce qu'on en
//! fait.
//!
//! Toute la matière vient de [`ygo_app::inventaire`], [`ygo_app::import`] et
//! [`ygo_app::export`], tous éprouvés en ligne de commande sur
//! l'installation réelle. Ici, il ne reste que les pixels — et deux règles
//! pures, [`intervalle`] et [`etiquette_variante`].
//!
//! # La sélection multiple est la raison d'être de cet écran
//!
//! Sur l'installation réelle, 330 cartes n'avaient aucun état parce
//! qu'elles avaient été saisies au clic dans le classeur. Les renseigner
//! une par une, c'est trois cent trente boîtes de dialogue. Le clic simple,
//! le `Ctrl+clic` et le `Maj+clic` désignent un lot, et l'action porte sur
//! tout le lot d'un coup.
//!
//! # La sélection qui ne sélectionnait pas — 2026-09-20
//!
//! Tout cela était écrit, testé… et inopérant à la souris. Les cellules du
//! tableau sont des `Label`, et egui rend les libellés **sélectionnables**
//! par défaut (`interaction.selectable_labels`) : chacun capte alors le clic
//! et le glisser pour la sélection de texte, **avant** la ligne qui est
//! dessous. Seul un clic dans le blanc d'une cellule atteignait la ligne ; un
//! clic sur le nom — là où tout le monde clique — ne faisait rien. Les tests
//! appelaient `cliquer` directement et passaient : ils éprouvaient la règle,
//! jamais le chemin du clic jusqu'à elle.
//!
//! Le tableau désactive désormais la sélection de texte dans son périmètre,
//! et s'aligne sur l'explorateur Windows, comme convenu :
//!
//! | Geste | Effet |
//! |---|---|
//! | clic | cette ligne seule |
//! | `Ctrl+clic` | ajoute ou retire la ligne |
//! | `Maj+clic` | l'intervalle depuis l'ancre, **à la place** de la sélection |
//! | `Ctrl+Maj+clic` | l'intervalle, **ajouté** à la sélection |
//! | glisser | l'intervalle balayé (avec `Ctrl` : ajouté) |
//! | `Ctrl+A` / `Échap` / `Suppr` | tout / rien / retirer… |
//! | clic droit | état, quantité, retrait — sur tout le lot |
//!
//! Un clic droit sur une ligne **hors** de la sélection la sélectionne seule
//! d'abord : c'est ce que fait Windows, et c'est ce qui évite d'agir sur un
//! lot qu'on ne regarde plus.
//!
//! # Pourquoi pas de sélecteur de fichier
//!
//! Ouvrir une vraie boîte de dialogue système demanderait `rfd`, qui tire
//! GTK sous Linux et une pile COM sous Windows — pour deux boutons. Le
//! fichier se **dépose** sur la fenêtre, ou son chemin s'écrit. C'est moins
//! joli et ça ne coûte rien à personne.

use std::collections::BTreeSet;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::csv::{Face, PanneauCsv};
use ygo_app::inventaire::{self, Carte, Colonne, Filtre};
use ygo_app::scanflip;
use ygo_core::paths::Paths;
use ygo_core::rarity::scanflip::{detail_qualite, guide_etats, libelle_qualite};

/// Largeur de la bande sensible au bord du tableau, en points.
///
/// Une hauteur de ligne et quelques points : assez pour qu'on y entre sans
/// viser, assez peu pour qu'on ne défile pas par mégarde en balayant la
/// dernière ligne visible.
pub const BANDE_DEFILEMENT: f32 = 28.0;

/// Vitesse maximale du défilement automatique, en points par image.
pub const VITESSE_DEFILEMENT: f32 = 22.0;

/// De combien faire défiler quand le pointeur approche d'un bord pendant un
/// glisser — et dans quel sens.
///
/// Rendu au format de `Ui::scroll_with_delta` : **négatif pour descendre**
/// dans la liste (le contenu monte), positif pour remonter. Zéro au milieu,
/// c'est-à-dire presque toujours.
///
/// La vitesse croît avec l'enfoncement dans la bande, et sature quand le
/// pointeur sort de la vue : frôler le bord fait défiler doucement, insister
/// fait défiler vite. Un défilement à vitesse fixe oblige à choisir entre
/// « trop lent pour sept cents lignes » et « impossible à arrêter à la
/// bonne ».
///
/// ```
/// use eframe::egui;
/// use ygo_ui::inventaire::{defilement_au_bord, VITESSE_DEFILEMENT};
/// let vue = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(500.0, 400.0));
/// // Au milieu : rien.
/// assert_eq!(defilement_au_bord(vue, 250.0), 0.0);
/// // Vers le bas : on descend dans la liste.
/// assert!(defilement_au_bord(vue, 395.0) < 0.0);
/// // Vers le haut : on remonte.
/// assert!(defilement_au_bord(vue, 105.0) > 0.0);
/// // Au-delà du bord, la vitesse sature.
/// assert_eq!(defilement_au_bord(vue, 600.0), -VITESSE_DEFILEMENT);
/// ```
#[must_use]
pub fn defilement_au_bord(vue: egui::Rect, y: f32) -> f32 {
    let vitesse =
        |profondeur: f32| (profondeur / BANDE_DEFILEMENT).clamp(0.0, 1.0) * VITESSE_DEFILEMENT;
    let haut = vue.top() + BANDE_DEFILEMENT;
    let bas = vue.bottom() - BANDE_DEFILEMENT;
    if y < haut {
        vitesse(haut - y)
    } else if y > bas {
        -vitesse(y - bas)
    } else {
        0.0
    }
}

/// Un lot de lignes, désignées par leur adresse complète `(classeur, rowid)`.
type Selection = BTreeSet<(String, i64)>;

/// L'intervalle de lignes qu'un `Maj+clic` désigne.
///
/// Rendu toujours croissant : sélectionner de bas en haut désigne le même
/// lot que de haut en bas. Sans cette normalisation, la moitié des
/// `Maj+clic` ne sélectionneraient rien.
///
/// ```
/// use ygo_ui::inventaire::intervalle;
/// assert_eq!(intervalle(2, 5), 2..=5);
/// assert_eq!(intervalle(5, 2), 2..=5);
/// assert_eq!(intervalle(3, 3), 3..=3);
/// ```
#[must_use]
pub fn intervalle(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
    if a <= b {
        a..=b
    } else {
        b..=a
    }
}

/// Ce qu'affiche la colonne « variante ».
///
/// Vide quand le tirage n'a qu'une illustration — la très grande majorité
/// des lignes. Écrire « Art 1/1 » partout ajouterait du bruit à sept cents
/// lignes pour renseigner sur quelques dizaines.
///
/// ```
/// use ygo_ui::inventaire::etiquette_variante;
/// assert_eq!(etiquette_variante(0, 1, false), "");
/// assert_eq!(etiquette_variante(2, 3, false), "art 2/3");
/// assert_eq!(etiquette_variante(0, 1, true), "Overframe");
/// ```
#[must_use]
pub fn etiquette_variante(variante: u32, variantes: u32, overframe: bool) -> String {
    match (variantes > 1, overframe) {
        (true, true) => format!("art {variante}/{variantes} · Overframe"),
        (true, false) => format!("art {variante}/{variantes}"),
        (false, true) => "Overframe".to_owned(),
        (false, false) => String::new(),
    }
}

/// Ce qu'un clic sur un en-tête de colonne fait au tri.
///
/// Recliquer la colonne déjà triée **renverse** le sens ; en choisir une
/// autre repart du sens montant — celui qu'on attend d'une colonne qu'on
/// découvre, et qui évite qu'un premier clic sur « Quantité » montre le
/// haut du tableau à l'envers.
///
/// ```
/// use ygo_app::inventaire::Colonne;
/// use ygo_ui::inventaire::basculer;
/// // Une autre colonne : montant.
/// assert_eq!(basculer(Colonne::Nom, true, Colonne::Quantite), (Colonne::Quantite, false));
/// // La même : on renverse.
/// assert_eq!(basculer(Colonne::Nom, false, Colonne::Nom), (Colonne::Nom, true));
/// assert_eq!(basculer(Colonne::Nom, true, Colonne::Nom), (Colonne::Nom, false));
/// ```
#[must_use]
pub fn basculer(actuelle: Colonne, descendant: bool, cliquee: Colonne) -> (Colonne, bool) {
    if actuelle == cliquee {
        (cliquee, !descendant)
    } else {
        (cliquee, false)
    }
}

/// L'écran de l'inventaire.
pub struct EcranInventaire {
    paths: Paths,
    francais: bool,

    cartes: Vec<Carte>,
    filtre: Filtre,
    colonne: Colonne,
    descendant: bool,

    /// Les lignes retenues, par leur adresse complète.
    selection: Selection,
    /// L'indice de la dernière ligne cliquée, pour le `Maj+clic`.
    ancre: Option<usize>,
    /// Un glisser en cours : la ligne de départ, et la sélection telle
    /// qu'elle était avant — `Some` seulement avec `Ctrl`, où le balayage
    /// **s'ajoute** à l'existant au lieu de le remplacer.
    glisse: Option<(usize, Option<Selection>)>,

    /// Valeurs proposées aux listes déroulantes, recalculées à chaque
    /// lecture de la base et non à chaque image.
    classeurs: Vec<String>,
    raretes: Vec<String>,
    etats: Vec<String>,

    /// L'état que l'action en masse écrira.
    etat_choisi: String,
    /// La quantité que l'action en masse écrira.
    quantite_choisie: i64,
    /// Un retrait attend confirmation.
    retrait: bool,

    /// L'import/export, partagé avec l'accueil.
    csv: PanneauCsv,
    /// Les classeurs dont l'utilisateur a demandé la création.
    ///
    /// L'écran ne les crée pas : la création va sur le réseau et vit sur
    /// le fil de travail de l'application, comme celle du sélecteur.
    creations: Vec<String>,

    /// Ce qu'il faut dire à l'utilisateur, et si c'est une erreur.
    message: Option<(String, bool)>,
    retour: bool,
    duree_lecture: std::time::Duration,
}

impl std::fmt::Debug for EcranInventaire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranInventaire")
            .field("cartes", &self.cartes.len())
            .field("selection", &self.selection.len())
            .finish_non_exhaustive()
    }
}

impl EcranInventaire {
    /// Lit l'inventaire d'une installation.
    #[must_use]
    pub fn ouvrir(paths: Paths, francais: bool) -> Self {
        let csv = PanneauCsv::nouveau("inventaire", &paths);
        let mut ecran = Self {
            paths,
            francais,
            cartes: Vec::new(),
            filtre: Filtre::default(),
            colonne: Colonne::default(),
            descendant: false,
            selection: BTreeSet::new(),
            ancre: None,
            glisse: None,
            classeurs: Vec::new(),
            raretes: Vec::new(),
            etats: Vec::new(),
            etat_choisi: "NM".to_owned(),
            quantite_choisie: 1,
            retrait: false,
            csv,
            creations: Vec::new(),
            message: None,
            retour: false,
            duree_lecture: std::time::Duration::ZERO,
        };
        ecran.relire();
        ecran
    }

    /// Relit la base et recalcule ce qui en dépend.
    fn relire(&mut self) {
        let debut = std::time::Instant::now();
        self.cartes = inventaire::lister(&self.paths);
        self.duree_lecture = debut.elapsed();
        self.classeurs = inventaire::valeurs(&self.cartes, |c| &c.classeur);
        self.raretes = inventaire::valeurs(&self.cartes, |c| &c.rarete);
        self.etats = inventaire::valeurs(&self.cartes, |c| &c.qualite);
        self.trier();
        // Une ligne retirée ne doit pas rester sélectionnée : l'action
        // suivante porterait sur une adresse qui n'est plus dans la liste.
        let vivantes: BTreeSet<(String, i64)> = self
            .cartes
            .iter()
            .map(|c| (c.classeur.clone(), c.rowid))
            .collect();
        self.selection.retain(|a| vivantes.contains(a));
        self.ancre = None;
        self.glisse = None;
    }

    fn trier(&mut self) {
        inventaire::trier(
            &mut self.cartes,
            self.colonne,
            self.descendant,
            self.francais,
        );
    }

    /// Reprend les créations de classeur demandées.
    pub fn creations_demandees(&mut self) -> Vec<String> {
        std::mem::take(&mut self.creations)
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// L'installation, pour l'appelant.
    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Les cartes visibles, filtre appliqué.
    fn visibles(&self) -> Vec<&Carte> {
        inventaire::filtrer(&self.cartes, &self.filtre, self.francais)
    }

    /// Applique une action en masse et rend un message.
    fn en_masse<F>(&mut self, quoi: &str, action: F)
    where
        F: FnOnce(&Paths, &[(String, i64)]) -> ygo_app::Result<inventaire::Touchees>,
    {
        let cibles: Vec<(String, i64)> = self.selection.iter().cloned().collect();
        if cibles.is_empty() {
            return;
        }
        match action(&self.paths, &cibles) {
            Ok(t) => {
                self.message = Some((
                    format!(
                        "{quoi} — {} ligne(s) dans {}",
                        t.lignes,
                        t.classeurs.join(", ")
                    ),
                    false,
                ));
                self.relire();
            }
            Err(e) => self.message = Some((e.to_string(), true)),
        }
    }
}

impl EcranInventaire {
    /// Écrit un état sur toute la sélection. Une chaîne vide l'efface.
    fn appliquer_etat(&mut self, etat: &str) {
        let quoi = if etat.is_empty() {
            "État effacé".to_owned()
        } else {
            format!("État « {etat} »")
        };
        let etat = etat.to_owned();
        self.en_masse(&quoi, |paths, cibles| {
            inventaire::definir_qualite(paths, cibles, &etat)
        });
    }

    /// Fixe la même quantité sur toute la sélection.
    fn appliquer_quantite(&mut self, n: i64) {
        self.en_masse(&format!("Quantité {n}"), |paths, cibles| {
            inventaire::definir_quantite(paths, cibles, n)
        });
    }

    /// Exécute ce que le menu contextuel a choisi.
    fn executer(&mut self, geste: Geste, visibles: &[Carte]) {
        match geste {
            Geste::Etat(etat) => self.appliquer_etat(&etat),
            Geste::Quantite(n) => self.appliquer_quantite(n),
            Geste::Retirer => self.retrait = true,
            Geste::CopierNumeros(ctx) => {
                let numeros: Vec<&str> = visibles
                    .iter()
                    .filter(|c| self.selection.contains(&(c.classeur.clone(), c.rowid)))
                    .map(|c| c.set_code.as_str())
                    .collect();
                ctx.copy_text(numeros.join("\n"));
                self.message = Some((format!("{} numéro(s) copié(s)", numeros.len()), false));
            }
            Geste::ToutSelectionner => self.tout_selectionner(visibles),
            Geste::Deselectionner => {
                self.selection.clear();
                self.ancre = None;
            }
        }
    }

    /// `Ctrl+A`, `Échap`, `Suppr` — seulement quand aucun champ de saisie
    /// n'a le focus : dans la recherche, `Ctrl+A` sélectionne le texte, pas
    /// sept cents lignes.
    fn raccourcis(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || self.retrait {
            return;
        }
        let (tout, rien, retirer) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::A),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            )
        });
        if tout {
            let visibles: Vec<Carte> = self.visibles().into_iter().cloned().collect();
            self.tout_selectionner(&visibles);
        }
        if rien {
            self.selection.clear();
            self.ancre = None;
        }
        if retirer && !self.selection.is_empty() {
            self.retrait = true;
        }
    }
}

/// Le lexique des états, pour l'infobulle du « ? ».
///
/// Les sept états du guide de notation de Scanflip, dans l'ordre de la page :
/// code, nom, et la phrase par laquelle Scanflip le définit. Le détail des
/// critères est dans l'infobulle de chaque entrée des listes.
#[must_use]
pub fn lexique_etats() -> String {
    let mut lignes = vec!["États — guide de notation de Scanflip".to_owned()];
    for e in guide_etats() {
        lignes.push(format!("{} — {}", e.code, e.nom));
        lignes.push(format!("    {}", e.resume()));
    }
    lignes.join("\n")
}

/// Un « ? » discret qui montre le lexique au survol.
fn bouton_lexique(ui: &mut egui::Ui) {
    ui.add(egui::Label::new(egui::RichText::new("?").weak()).sense(egui::Sense::hover()))
        .on_hover_text(egui::RichText::new(lexique_etats()).monospace());
}

/// Ce que le menu contextuel demande, exécuté **après** le dessin du tableau
/// pour ne pas muter la sélection qu'on est en train de parcourir.
#[derive(Debug, Clone)]
enum Geste {
    Etat(String),
    Quantite(i64),
    Retirer,
    CopierNumeros(egui::Context),
    ToutSelectionner,
    Deselectionner,
}

/// Le menu d'un clic droit : il agit sur **toute** la sélection.
fn menu_selection(ui: &mut egui::Ui, n: usize, geste: &mut Option<Geste>) {
    ui.label(egui::RichText::new(format!("{n} carte(s) sélectionnée(s)")).strong());
    ui.separator();
    ui.menu_button("État", |ui| {
        for e in scanflip::ETATS {
            let bouton = ui.button(libelle_qualite(e));
            let bouton = match detail_qualite(e) {
                Some(texte) => bouton.on_hover_text(texte),
                None => bouton,
            };
            if bouton.clicked() {
                *geste = Some(Geste::Etat(e.to_owned()));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("(effacer)").clicked() {
            *geste = Some(Geste::Etat(String::new()));
            ui.close();
        }
    });
    ui.menu_button("Quantité", |ui| {
        for q in 1..=3 {
            let libelle = if q == 3 {
                "3 (playset)".to_owned()
            } else {
                q.to_string()
            };
            if ui.button(libelle).clicked() {
                *geste = Some(Geste::Quantite(q));
                ui.close();
            }
        }
        ui.separator();
        ui.small("Autre valeur : barre du bas");
    });
    ui.separator();
    if ui.button("Copier les numéros").clicked() {
        *geste = Some(Geste::CopierNumeros(ui.ctx().clone()));
        ui.close();
    }
    if ui.button("Tout sélectionner  (Ctrl+A)").clicked() {
        *geste = Some(Geste::ToutSelectionner);
        ui.close();
    }
    if ui.button("Désélectionner  (Échap)").clicked() {
        *geste = Some(Geste::Deselectionner);
        ui.close();
    }
    ui.separator();
    if ui.button("Retirer de l'inventaire…  (Suppr)").clicked() {
        *geste = Some(Geste::Retirer);
        ui.close();
    }
}

impl eframe::App for EcranInventaire {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let paths = self.paths.clone();
        if self.csv.capter_depot(racine.ctx(), &paths) {
            self.message = None;
        }
        self.raccourcis(racine.ctx());
        self.barre_du_haut(racine);
        self.barre_des_filtres(racine);
        self.barre_du_bas(racine);
        let retour = self.csv.afficher(racine, &paths, self.francais);
        self.appliquer_retour(retour);
        self.table(racine);
        self.confirmation_retrait(racine.ctx());
    }
}

impl EcranInventaire {
    /// Tire les conséquences de ce que le panneau CSV a fait.
    ///
    /// Un import appliqué change les quantités : l'inventaire doit se
    /// relire, sans quoi il montrerait l'état d'avant.
    fn appliquer_retour(&mut self, retour: crate::csv::Retour) {
        if retour.vide() {
            return;
        }
        if let Some(m) = retour.message {
            self.message = Some(m);
        }
        if !retour.creations.is_empty() {
            self.creations = retour.creations;
        }
        if retour.importe {
            self.relire();
        }
    }

    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-inventaire").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Mes classeurs").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Toutes mes cartes");
                ui.add_space(12.0);

                let visibles = self.visibles();
                let t = inventaire::totaux(&visibles);
                ui.label(format!(
                    "{} ligne(s) · {} exemplaire(s) · {} playset(s) · {} en surplus",
                    t.lignes, t.exemplaires, t.playsets, t.surplus
                ));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Exporter…").clicked() {
                        self.csv.basculer(Face::Export);
                    }
                    ui.add_space(8.0);
                    if ui.button("Importer…").clicked() {
                        self.csv.basculer(Face::Import);
                    }
                    ui.add_space(8.0);
                    if ui.checkbox(&mut self.francais, "FR").changed() {
                        self.trier();
                    }
                });
            });
            if let Some((texte, erreur)) = self.message.clone() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let couleur = if erreur {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().hyperlink_color
                    };
                    ui.colored_label(couleur, texte);
                    if ui.small_button("✖").clicked() {
                        self.message = None;
                    }
                });
            }
            ui.add_space(8.0);
        });
    }

    fn barre_des_filtres(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("filtres-inventaire").show(racine, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filtre.nom)
                        .hint_text("nom…")
                        .desired_width(200.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.filtre.code)
                        .hint_text("code…")
                        .desired_width(120.0),
                );
                deroulante(
                    ui,
                    "classeur",
                    "Classeur",
                    &self.classeurs,
                    &mut self.filtre.classeur,
                );
                deroulante(
                    ui,
                    "rarete",
                    "Rareté",
                    &self.raretes,
                    &mut self.filtre.rarete,
                );

                // L'état a une entrée de plus que les valeurs présentes :
                // « sans état », qui ne peut pas se demander par la chaîne
                // vide — celle-ci veut dire « ne pas filtrer ».
                let libelle = if self.filtre.qualite == Filtre::SANS_QUALITE {
                    "sans état".to_owned()
                } else if self.filtre.qualite.is_empty() {
                    "État".to_owned()
                } else {
                    libelle_qualite(&self.filtre.qualite)
                };
                egui::ComboBox::from_id_salt("etat-inventaire")
                    .selected_text(libelle)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filtre.qualite, String::new(), "État");
                        ui.selectable_value(
                            &mut self.filtre.qualite,
                            Filtre::SANS_QUALITE.to_owned(),
                            "sans état",
                        );
                        ui.separator();
                        for v in &self.etats {
                            ui.selectable_value(
                                &mut self.filtre.qualite,
                                v.clone(),
                                libelle_qualite(v),
                            );
                        }
                    });

                ui.checkbox(&mut self.filtre.sous_playset, "playset incomplet");
                if self.filtre.actif() && ui.button("Tout montrer").clicked() {
                    self.filtre = Filtre::default();
                }
            });
            ui.add_space(6.0);
        });
    }

    /// La barre du bas : ce que la sélection permet de faire.
    ///
    /// Elle n'apparaît qu'avec une sélection. Des boutons grisés en
    /// permanence occuperaient la place sans jamais renseigner.
    fn barre_du_bas(&mut self, racine: &mut egui::Ui) {
        egui::Panel::bottom("actions-inventaire").show(racine, |ui| {
            ui.add_space(6.0);
            if self.selection.is_empty() {
                ui.horizontal(|ui| {
                    ui.small(format!(
                        "{} carte(s) lues en {:?} — clic, Ctrl+clic, Maj+clic ou glisser \
                         pour sélectionner ; Ctrl+A pour tout ; clic droit pour agir",
                        self.cartes.len(),
                        self.duree_lecture
                    ));
                });
                ui.add_space(6.0);
                return;
            }

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} sélectionnée(s)", self.selection.len()))
                        .strong(),
                );
                if ui.button("Désélectionner").clicked() {
                    self.selection.clear();
                    self.ancre = None;
                }
                ui.separator();

                ui.label("État");
                bouton_lexique(ui);
                egui::ComboBox::from_id_salt("etat-masse")
                    .selected_text(if self.etat_choisi.is_empty() {
                        "(effacer)".to_owned()
                    } else {
                        libelle_qualite(&self.etat_choisi)
                    })
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for e in scanflip::ETATS {
                            let entree = ui.selectable_value(
                                &mut self.etat_choisi,
                                e.to_owned(),
                                libelle_qualite(e),
                            );
                            if let Some(texte) = detail_qualite(e) {
                                entree.on_hover_text(texte);
                            }
                        }
                        ui.separator();
                        ui.selectable_value(&mut self.etat_choisi, String::new(), "(effacer)");
                    });
                if ui.button("Appliquer").clicked() {
                    let etat = self.etat_choisi.clone();
                    self.appliquer_etat(&etat);
                }
                ui.separator();

                ui.label("Quantité");
                ui.add(
                    egui::DragValue::new(&mut self.quantite_choisie)
                        .range(0..=99)
                        .speed(0.1),
                );
                if ui.button("Fixer").clicked() {
                    self.appliquer_quantite(self.quantite_choisie);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Retirer de l'inventaire…").clicked() {
                        self.retrait = true;
                    }
                });
            });
            ui.add_space(6.0);
        });
    }

    /// Retirer, ce n'est pas effacer — mais ça se confirme quand même.
    fn confirmation_retrait(&mut self, ctx: &egui::Context) {
        if !self.retrait {
            return;
        }
        let combien = self.selection.len();
        let exemplaires: i64 = self
            .cartes
            .iter()
            .filter(|c| self.selection.contains(&(c.classeur.clone(), c.rowid)))
            .map(|c| c.quantite)
            .sum();
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new(format!("Retirer {combien} carte(s) ?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("{exemplaires} exemplaire(s) ne seront plus comptés."),
                );
                ui.add_space(6.0);
                ui.label(
                    "Les lignes restent dans leurs classeurs : le tirage existe \
                     toujours, il n'est simplement plus possédé. Vous pouvez le \
                     remettre en fixant une quantité.",
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.retrait = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Retirer").clicked() {
                            confirme = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });
        if confirme {
            self.retrait = false;
            self.en_masse("Retirées", inventaire::retirer);
        } else if !ouverte {
            self.retrait = false;
        }
    }

    fn table(&mut self, racine: &mut egui::Ui) {
        egui::CentralPanel::default().show(racine, |ui| {
            let visibles: Vec<Carte> = self.visibles().into_iter().cloned().collect();
            if visibles.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    if self.cartes.is_empty() {
                        ui.heading("Aucune carte possédée.");
                        ui.label(
                            "Ouvrez un classeur pour y saisir des quantités, ou importez un CSV.",
                        );
                    } else {
                        ui.heading("Aucune carte ne correspond aux filtres.");
                        ui.add_space(8.0);
                        if ui.button("Tout montrer").clicked() {
                            self.filtre = Filtre::default();
                        }
                    }
                });
                return;
            }

            let mut a_trier: Option<Colonne> = None;
            let mut clic: Option<usize> = None;
            let mut droit: Option<usize> = None;
            let mut debut_glisse: Option<usize> = None;
            let mut sous_pointeur: Option<usize> = None;
            let mut geste: Option<Geste> = None;
            let (ctrl, maj, pointeur, enfonce) = ui.input(|i| {
                (
                    i.modifiers.command,
                    i.modifiers.shift,
                    i.pointer.interact_pos(),
                    i.pointer.primary_down(),
                )
            });
            let n_selection = self.selection.len();
            // Le glisser en cours, pour le défilement automatique : il se
            // déclenche dans une cellule, seul endroit d'où l'on puisse
            // atteindre la zone défilante du tableau.
            let glisse_active = self.glisse.is_some() && enfonce;
            let defilement_fait = std::cell::Cell::new(false);

            // La cause du défaut du 2026-09-20 : un libellé sélectionnable
            // capte le clic avant la ligne qui est dessous. Dans le tableau,
            // on sélectionne des lignes, pas du texte.
            ui.style_mut().interaction.selectable_labels = false;

            TableBuilder::new(ui)
                .striped(true)
                .sense(egui::Sense::click_and_drag())
                .column(Column::auto().at_least(220.0).resizable(true))
                .column(Column::auto().at_least(120.0))
                .column(Column::auto().at_least(150.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(60.0))
                .column(Column::auto().at_least(70.0))
                .column(Column::auto().at_least(90.0))
                .column(Column::remainder())
                .header(24.0, |mut entete| {
                    for colonne in [
                        Colonne::Nom,
                        Colonne::Code,
                        Colonne::Rarete,
                        Colonne::Quantite,
                        Colonne::Surplus,
                        Colonne::Qualite,
                        Colonne::Classeur,
                    ] {
                        entete.col(|ui| {
                            // La flèche est écrite en mots ailleurs dans le
                            // projet parce que ▲/▼ manquent aux polices
                            // d'egui ; ici, deux caractères que les polices
                            // ont bien.
                            let marque = if self.colonne == colonne {
                                if self.descendant {
                                    " ↓"
                                } else {
                                    " ↑"
                                }
                            } else {
                                ""
                            };
                            if ui
                                .button(
                                    egui::RichText::new(format!("{}{marque}", colonne.libelle()))
                                        .strong(),
                                )
                                .clicked()
                            {
                                a_trier = Some(colonne);
                            }
                        });
                    }
                    entete.col(|ui| {
                        ui.label(egui::RichText::new("Variante").strong());
                    });
                })
                .body(|corps| {
                    corps.rows(22.0, visibles.len(), |mut ligne| {
                        let i = ligne.index();
                        let Some(c) = visibles.get(i) else {
                            return;
                        };
                        let adresse = (c.classeur.clone(), c.rowid);
                        ligne.set_selected(self.selection.contains(&adresse));

                        ligne.col(|ui| {
                            if glisse_active && !defilement_fait.replace(true) {
                                if let Some(p) = pointeur {
                                    let pas = defilement_au_bord(ui.clip_rect(), p.y);
                                    if pas != 0.0 {
                                        // Sans animation : pendant un glisser,
                                        // l'inertie fait dépasser la ligne visée.
                                        ui.scroll_with_delta_animation(
                                            egui::vec2(0.0, pas),
                                            egui::style::ScrollAnimation::none(),
                                        );
                                        // Le pointeur peut rester immobile au
                                        // bord : sans cela, l'image suivante
                                        // n'arriverait jamais.
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }
                            ui.label(c.nom_affiche(self.francais));
                        });
                        ligne.col(|ui| {
                            ui.label(egui::RichText::new(c.set_code.as_str()).monospace());
                        });
                        ligne.col(|ui| {
                            ui.label(c.rarete.as_str());
                        });
                        ligne.col(|ui| {
                            ui.label(format!("{}", c.quantite));
                        });
                        ligne.col(|ui| {
                            if c.surplus() > 0 {
                                ui.label(format!("+{}", c.surplus()));
                            }
                        });
                        ligne.col(|ui| {
                            if c.qualite.is_empty() {
                                ui.weak("—");
                            } else {
                                // La colonne reste étroite : le code, et le
                                // nom complet au survol.
                                let r = ui.label(c.qualite.as_str());
                                if let Some(texte) = detail_qualite(&c.qualite) {
                                    r.on_hover_text(texte);
                                }
                            }
                        });
                        ligne.col(|ui| {
                            ui.label(c.classeur.as_str());
                        });
                        ligne.col(|ui| {
                            let etiquette =
                                etiquette_variante(c.variante, c.variantes, c.overframe);
                            if !etiquette.is_empty() {
                                ui.weak(etiquette);
                            }
                        });

                        let reponse = ligne.response();
                        if reponse.clicked() {
                            clic = Some(i);
                        }
                        if reponse.secondary_clicked() {
                            droit = Some(i);
                        }
                        if reponse.drag_started_by(egui::PointerButton::Primary) {
                            debut_glisse = Some(i);
                        }
                        // Pendant un glisser, c'est la ligne de départ qui
                        // « tient » le pointeur : la ligne survolée se lit
                        // donc sur les rectangles, pas sur le survol.
                        if enfonce && pointeur.is_some_and(|p| reponse.rect.contains(p)) {
                            sous_pointeur = Some(i);
                        }
                        reponse.context_menu(|ui| {
                            menu_selection(ui, n_selection.max(1), &mut geste);
                        });
                    });
                });

            if let Some(colonne) = a_trier {
                let (c, d) = basculer(self.colonne, self.descendant, colonne);
                self.colonne = c;
                self.descendant = d;
                self.trier();
                self.ancre = None;
            }

            if let Some(i) = clic {
                self.cliquer(&visibles, i, ctrl, maj);
            }
            if let Some(i) = droit {
                self.clic_droit(&visibles, i);
            }
            if let Some(i) = debut_glisse {
                self.commencer_glisse(i, ctrl);
            }
            if let (Some((depart, base)), Some(i)) = (self.glisse.clone(), sous_pointeur) {
                self.glisser(&visibles, depart, base.as_ref(), i);
            }
            if !enfonce {
                self.glisse = None;
            }
            if let Some(g) = geste {
                self.executer(g, &visibles);
            }
        });
    }

    /// `Ctrl+A` : toutes les lignes **visibles** — le filtre délimite ce
    /// qu'on regarde, et c'est cela qu'on veut prendre.
    fn tout_selectionner(&mut self, visibles: &[Carte]) {
        self.selection = visibles
            .iter()
            .map(|c| (c.classeur.clone(), c.rowid))
            .collect();
        self.ancre = None;
    }

    /// Un clic droit hors de la sélection la remplace par cette ligne ; dans
    /// la sélection, il la laisse telle quelle. C'est le comportement de
    /// l'explorateur Windows.
    fn clic_droit(&mut self, visibles: &[Carte], i: usize) {
        let Some(c) = visibles.get(i) else {
            return;
        };
        let adresse = (c.classeur.clone(), c.rowid);
        if !self.selection.contains(&adresse) {
            self.selection.clear();
            self.selection.insert(adresse);
            self.ancre = Some(i);
        }
    }

    /// Le bouton vient d'être enfoncé sur une ligne et le pointeur bouge.
    fn commencer_glisse(&mut self, depart: usize, ctrl: bool) {
        let base = ctrl.then(|| self.selection.clone());
        self.glisse = Some((depart, base));
        self.ancre = Some(depart);
    }

    /// Le balayage en cours : l'intervalle entre le départ et la ligne sous
    /// le pointeur — seul, ou ajouté à la sélection d'avant si `Ctrl` était
    /// tenu au départ. Recalculé à chaque image, si bien que revenir en
    /// arrière désélectionne ce qu'on a dépassé.
    fn glisser(
        &mut self,
        visibles: &[Carte],
        depart: usize,
        base: Option<&Selection>,
        courante: usize,
    ) {
        let mut selection = base.cloned().unwrap_or_default();
        for j in intervalle(depart, courante) {
            if let Some(v) = visibles.get(j) {
                selection.insert((v.classeur.clone(), v.rowid));
            }
        }
        self.selection = selection;
    }

    /// Ce qu'un clic fait à la sélection.
    ///
    /// Clic simple : cette ligne seule. `Ctrl+clic` : ajoute ou retire.
    /// `Maj+clic` : l'intervalle depuis l'ancre, à la place de la sélection ;
    /// avec `Ctrl` en plus, ajouté à elle. L'ancre ne bouge pas sous `Maj`,
    /// si bien que deux `Maj+clic` successifs ajustent le même intervalle —
    /// comme dans l'explorateur.
    fn cliquer(&mut self, visibles: &[Carte], i: usize, ctrl: bool, maj: bool) {
        let Some(c) = visibles.get(i) else {
            return;
        };
        let adresse = (c.classeur.clone(), c.rowid);
        match (ctrl, maj, self.ancre) {
            (_, true, Some(ancre)) => {
                if !ctrl {
                    self.selection.clear();
                }
                for j in intervalle(ancre, i) {
                    if let Some(v) = visibles.get(j) {
                        self.selection.insert((v.classeur.clone(), v.rowid));
                    }
                }
            }
            (true, _, _) => {
                if !self.selection.remove(&adresse) {
                    self.selection.insert(adresse);
                }
                self.ancre = Some(i);
            }
            _ => {
                self.selection.clear();
                self.selection.insert(adresse);
                self.ancre = Some(i);
            }
        }
    }
}

/// Une liste déroulante de valeurs, avec une entrée « tout » en tête.
fn deroulante(ui: &mut egui::Ui, id: &str, titre: &str, valeurs: &[String], choix: &mut String) {
    let libelle = if choix.is_empty() {
        titre
    } else {
        choix.as_str()
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(libelle)
        .width(150.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(choix, String::new(), titre);
            ui.separator();
            for v in valeurs {
                ui.selectable_value(choix, v.clone(), v);
            }
        });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn carte(classeur: &str, rowid: i64) -> Carte {
        Carte {
            classeur: classeur.to_owned(),
            rowid,
            nom: format!("Card {rowid}"),
            nom_fr: String::new(),
            set_nom: String::new(),
            set_code: format!("{classeur}-EN{rowid:03}"),
            rarete: "Common".into(),
            quantite: 1,
            qualite: String::new(),
            overframe: false,
            card_image_id: Some(rowid),
            variantes: 1,
            variante: 0,
        }
    }

    fn ecran(cartes: Vec<Carte>) -> EcranInventaire {
        let tmp = tempfile::tempdir().unwrap();
        let mut e = EcranInventaire::ouvrir(Paths::depuis_racine(tmp.path()), true);
        e.cartes = cartes;
        e
    }

    /// L'intervalle du `Maj+clic` est le même dans les deux sens.
    #[test]
    fn l_intervalle_est_le_meme_de_haut_en_bas_et_de_bas_en_haut() {
        assert_eq!(intervalle(2, 5).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        assert_eq!(intervalle(5, 2).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        assert_eq!(intervalle(4, 4).collect::<Vec<_>>(), vec![4]);
    }

    /// Le clic simple remplace la sélection ; `Ctrl` l'étend et la réduit ;
    /// `Maj` prend l'intervalle.
    #[test]
    fn les_trois_gestes_de_selection_font_trois_choses_distinctes() {
        let visibles: Vec<Carte> = (1..=5).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());

        // Clic simple : une seule.
        e.cliquer(&visibles, 1, false, false);
        assert_eq!(e.selection.len(), 1);
        assert!(e.selection.contains(&("RA02".to_owned(), 2)));

        // Clic simple ailleurs : remplace, n'ajoute pas.
        e.cliquer(&visibles, 3, false, false);
        assert_eq!(e.selection.len(), 1);
        assert!(e.selection.contains(&("RA02".to_owned(), 4)));

        // Ctrl+clic : ajoute.
        e.cliquer(&visibles, 0, true, false);
        assert_eq!(e.selection.len(), 2);
        // Ctrl+clic sur une ligne déjà prise : retire.
        e.cliquer(&visibles, 0, true, false);
        assert_eq!(e.selection.len(), 1);

        // Maj+clic depuis l'ancre : tout l'intervalle.
        e.selection.clear();
        e.cliquer(&visibles, 1, false, false);
        e.cliquer(&visibles, 4, false, true);
        assert_eq!(e.selection.len(), 4, "de la 2e à la 5e");
        for rowid in 2..=5 {
            assert!(e.selection.contains(&("RA02".to_owned(), rowid)));
        }
    }

    /// Windows : `Maj+clic` **remplace** la sélection par l'intervalle ;
    /// `Ctrl+Maj+clic` l'y **ajoute**.
    #[test]
    fn maj_remplace_et_ctrl_maj_ajoute() {
        let visibles: Vec<Carte> = (1..=8).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 6, false, false); // la 7e, seule
        e.cliquer(&visibles, 0, false, false); // ancre sur la 1re
        e.cliquer(&visibles, 2, false, true); // Maj : 1re → 3e
        assert_eq!(e.selection.len(), 3, "la 7e n'y est plus");
        assert!(!e.selection.contains(&("RA02".to_owned(), 7)));

        e.cliquer(&visibles, 6, true, false); // Ctrl : +7e, ancre sur elle
        e.cliquer(&visibles, 7, true, true); // Ctrl+Maj : +7e→8e
        assert_eq!(e.selection.len(), 5, "1-3 gardées, 7-8 ajoutées");
    }

    /// Deux `Maj+clic` de suite ajustent le même intervalle : l'ancre reste
    /// où le dernier clic simple l'a posée.
    #[test]
    fn deux_maj_clics_ajustent_le_meme_intervalle() {
        let visibles: Vec<Carte> = (1..=6).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 1, false, false);
        e.cliquer(&visibles, 5, false, true);
        assert_eq!(e.selection.len(), 5);
        e.cliquer(&visibles, 3, false, true);
        assert_eq!(e.selection.len(), 3, "2e → 4e, pas 2e → 6e");
    }

    /// `Ctrl+A` prend ce qu'on voit — pas ce que le filtre a caché.
    #[test]
    fn tout_selectionner_prend_les_lignes_visibles() {
        let toutes: Vec<Carte> = (1..=5).map(|i| carte("RA02", i)).collect();
        let visibles: Vec<Carte> = toutes[..3].to_vec();
        let mut e = ecran(toutes);
        e.tout_selectionner(&visibles);
        assert_eq!(e.selection.len(), 3);
    }

    /// Clic droit hors de la sélection : cette ligne seule. Dans la
    /// sélection : on n'y touche pas — c'est sur elle que le menu agira.
    #[test]
    fn le_clic_droit_suit_l_explorateur() {
        let visibles: Vec<Carte> = (1..=5).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 0, false, false);
        e.cliquer(&visibles, 2, false, true);
        assert_eq!(e.selection.len(), 3);

        e.clic_droit(&visibles, 1);
        assert_eq!(e.selection.len(), 3, "dans la sélection : intacte");

        e.clic_droit(&visibles, 4);
        assert_eq!(e.selection.len(), 1, "hors sélection : remplacée");
        assert!(e.selection.contains(&("RA02".to_owned(), 5)));
    }

    /// Le glisser balaie un intervalle, et revenir en arrière rend ce qu'on
    /// avait dépassé.
    #[test]
    fn le_glisser_balaie_et_se_retracte() {
        let visibles: Vec<Carte> = (1..=6).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.commencer_glisse(1, false);
        let (depart, base) = e.glisse.clone().unwrap();
        e.glisser(&visibles, depart, base.as_ref(), 4);
        assert_eq!(e.selection.len(), 4, "2e → 5e");
        e.glisser(&visibles, depart, base.as_ref(), 2);
        assert_eq!(e.selection.len(), 2, "revenu à la 3e : 2e → 3e");
        e.glisser(&visibles, depart, base.as_ref(), 0);
        assert_eq!(e.selection.len(), 2, "vers le haut aussi : 1re → 2e");
    }

    /// Avec `Ctrl` au départ, le balayage s'ajoute à ce qui était pris.
    #[test]
    fn le_glisser_avec_ctrl_ajoute() {
        let visibles: Vec<Carte> = (1..=6).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 5, false, false);
        e.commencer_glisse(0, true);
        let (depart, base) = e.glisse.clone().unwrap();
        e.glisser(&visibles, depart, base.as_ref(), 1);
        assert_eq!(e.selection.len(), 3, "la 6e gardée, 1re et 2e ajoutées");
    }

    /// Le lexique suit le guide de Scanflip : les sept états, dans l'ordre de
    /// la page, chacun avec sa phrase.
    #[test]
    fn le_lexique_suit_le_guide_de_scanflip() {
        let lexique = lexique_etats();
        let entetes: Vec<&str> = lexique
            .lines()
            .skip(1)
            .filter(|l| !l.starts_with(' '))
            .collect();
        assert_eq!(entetes.len(), scanflip::ETATS.len());
        assert_eq!(entetes[0], "M — Mint");
        assert_eq!(entetes[1], "NM — Near Mint");
        assert_eq!(
            entetes[4], "PL — Played",
            "le nom du guide, pas de l'import"
        );
        assert_eq!(entetes[6], "DM — Damaged");
        assert!(lexique.contains("très proche du neuf"));
    }

    /// Le défilement au bord : sens, saturation, et silence au milieu.
    #[test]
    fn le_defilement_suit_le_bord_approche() {
        let vue = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(500.0, 400.0));

        // Tout le centre est muet — c'est le cas de presque tout un balayage.
        for y in [130.0, 200.0, 250.0, 300.0, 370.0] {
            assert_eq!(defilement_au_bord(vue, y), 0.0, "y = {y}");
        }

        // Plus on s'enfonce, plus c'est rapide. La bande basse commence à
        // 372 : 380 vient d'y entrer, 395 est presque au bord.
        let frole = defilement_au_bord(vue, 380.0);
        let insiste = defilement_au_bord(vue, 395.0);
        assert!(frole < 0.0 && insiste < frole, "{frole} puis {insiste}");

        // Et la vitesse sature : sortir de la fenêtre n'emballe pas la liste.
        assert_eq!(defilement_au_bord(vue, 400.0), -VITESSE_DEFILEMENT);
        assert_eq!(defilement_au_bord(vue, 10_000.0), -VITESSE_DEFILEMENT);
        assert_eq!(defilement_au_bord(vue, 100.0), VITESSE_DEFILEMENT);
        assert_eq!(defilement_au_bord(vue, -10_000.0), VITESSE_DEFILEMENT);

        // Les deux sens sont symétriques.
        assert_eq!(
            defilement_au_bord(vue, 100.0 + 8.0),
            -defilement_au_bord(vue, 400.0 - 8.0)
        );
    }

    /// Une vue plus courte que deux bandes ne doit pas défiler dans les deux
    /// sens à la fois ni diviser par zéro — le cas d'une fenêtre écrasée.
    #[test]
    fn une_vue_minuscule_ne_s_affole_pas() {
        let vue = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 10.0));
        let pas = defilement_au_bord(vue, 5.0);
        assert!(pas.is_finite());
        assert!(pas.abs() <= VITESSE_DEFILEMENT);
    }

    /// Clique **sur le texte** de la première ligne d'un vrai tableau egui,
    /// dans un contexte sans fenêtre, et dit si la ligne l'a reçu.
    ///
    /// C'est le chemin que les autres tests ne prenaient pas : eux appellent
    /// `cliquer` directement. Le défaut du 2026-09-20 vivait entre le pointeur
    /// et `cliquer`.
    fn la_ligne_recoit_un_clic_sur_son_texte(selectionnables: bool) -> bool {
        use std::cell::Cell;
        let ctx = egui::Context::default();
        let texte = Cell::new(egui::Rect::NOTHING);
        let recu = Cell::new(false);
        let image = |evenements: Vec<egui::Event>| {
            let entree = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: evenements,
                ..Default::default()
            };
            let mut sortie = ctx.run_ui(entree, |racine| {
                egui::CentralPanel::default().show(racine, |ui| {
                    ui.style_mut().interaction.selectable_labels = selectionnables;
                    TableBuilder::new(ui)
                        .sense(egui::Sense::click_and_drag())
                        .column(Column::exact(300.0))
                        .body(|corps| {
                            corps.rows(22.0, 1, |mut ligne| {
                                ligne.col(|ui| {
                                    texte.set(ui.label("Droll & Lock Bird").rect);
                                });
                                if ligne.response().clicked() {
                                    recu.set(true);
                                }
                            });
                        });
                });
            });
            // Sans moteur de rendu, personne ne consomme les textures.
            sortie.textures_delta.clear();
        };
        image(Vec::new());
        let p = texte.get().center();
        let bouton = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        image(vec![egui::Event::PointerMoved(p)]);
        image(vec![bouton(true)]);
        image(vec![bouton(false)]);
        image(Vec::new());
        recu.get()
    }

    /// La cause, démontrée : un libellé sélectionnable garde le clic pour
    /// lui ; sans sélection de texte, la ligne le reçoit.
    #[test]
    fn un_clic_sur_le_nom_atteint_la_ligne() {
        assert!(
            !la_ligne_recoit_un_clic_sur_son_texte(true),
            "le défaut : le libellé capte le clic"
        );
        assert!(
            la_ligne_recoit_un_clic_sur_son_texte(false),
            "le correctif : la ligne le reçoit"
        );
    }

    /// Un `Maj+clic` sans ancre se comporte comme un clic simple plutôt que
    /// de ne rien faire.
    #[test]
    fn un_maj_clic_sans_ancre_selectionne_la_ligne() {
        let visibles: Vec<Carte> = (1..=3).map(|i| carte("RA02", i)).collect();
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 2, false, true);
        assert_eq!(e.selection.len(), 1);
        assert!(e.selection.contains(&("RA02".to_owned(), 3)));
    }

    /// La sélection porte l'adresse complète : deux classeurs ayant chacun
    /// une ligne 1 ne se confondent pas.
    #[test]
    fn deux_classeurs_aux_rowid_identiques_se_selectionnent_separement() {
        let visibles = vec![carte("RA02", 1), carte("SDLI", 1)];
        let mut e = ecran(visibles.clone());
        e.cliquer(&visibles, 0, false, false);
        e.cliquer(&visibles, 1, true, false);
        assert_eq!(e.selection.len(), 2, "deux adresses distinctes");
        assert!(e.selection.contains(&("RA02".to_owned(), 1)));
        assert!(e.selection.contains(&("SDLI".to_owned(), 1)));
    }

    /// Le clic sur un en-tête : renverser la colonne courante, repartir
    /// montant sur une autre.
    #[test]
    fn recliquer_une_colonne_renverse_le_sens() {
        // Une autre colonne : toujours montant, même si on descendait.
        for descendant in [false, true] {
            assert_eq!(
                basculer(Colonne::Nom, descendant, Colonne::Quantite),
                (Colonne::Quantite, false)
            );
        }
        // La même colonne : le sens s'inverse, dans les deux sens.
        assert_eq!(
            basculer(Colonne::Quantite, false, Colonne::Quantite),
            (Colonne::Quantite, true)
        );
        assert_eq!(
            basculer(Colonne::Quantite, true, Colonne::Quantite),
            (Colonne::Quantite, false)
        );
        // Deux clics de suite ramènent à l'état de départ.
        let (c, d) = basculer(Colonne::Nom, false, Colonne::Nom);
        assert_eq!(basculer(c, d, Colonne::Nom), (Colonne::Nom, false));
    }

    /// L'étiquette de variante ne dit rien quand il n'y a rien à dire.
    #[test]
    fn l_etiquette_de_variante_se_tait_sur_un_tirage_unique() {
        assert_eq!(etiquette_variante(0, 1, false), "");
        assert_eq!(etiquette_variante(1, 2, false), "art 1/2");
        assert_eq!(etiquette_variante(2, 2, true), "art 2/2 · Overframe");
        assert_eq!(etiquette_variante(0, 1, true), "Overframe");
    }

    /// Relire la base oublie les lignes qui ont disparu de la sélection.
    ///
    /// Sans quoi une action suivante porterait sur une adresse qui n'est
    /// plus dans la liste — et le compte affiché mentirait.
    #[test]
    fn la_selection_oublie_les_lignes_disparues() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for rowid in 1..=2 {
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite)
                 VALUES (?1, 'Carte', 'RA02-EN001', 'Common', 1, 1)",
                [rowid],
            )
            .unwrap();
        }
        drop(conn);

        let mut e = EcranInventaire::ouvrir(paths.clone(), true);
        assert_eq!(e.cartes.len(), 2);
        e.selection.insert(("RA02".to_owned(), 1));
        e.selection.insert(("RA02".to_owned(), 2));

        // La ligne 2 est retirée de l'inventaire par ailleurs.
        ygo_app::inventaire::retirer(&paths, &[("RA02".to_owned(), 2)]).unwrap();
        e.relire();

        assert_eq!(e.cartes.len(), 1);
        assert_eq!(e.selection.len(), 1, "la ligne retirée quitte la sélection");
        assert!(e.selection.contains(&("RA02".to_owned(), 1)));
    }

    /// Les valeurs des listes déroulantes viennent de la base, pas d'une
    /// liste écrite en dur.
    #[test]
    fn les_listes_deroulantes_se_remplissent_de_ce_qui_existe() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity, qualite, possessed, quantite)
             VALUES (1, 'Carte', 'RA02-EN001', 'Secret Rare', 'NM', 1, 1)",
            (),
        )
        .unwrap();
        drop(conn);

        let e = EcranInventaire::ouvrir(paths, true);
        assert_eq!(e.classeurs, vec!["RA02".to_owned()]);
        assert_eq!(e.raretes, vec!["Secret Rare".to_owned()]);
        assert_eq!(e.etats, vec!["NM".to_owned()]);
    }

    /// Le retour se consomme une seule fois.
    #[test]
    fn le_retour_se_consomme() {
        let mut e = ecran(Vec::new());
        e.retour = true;
        assert!(e.retour_demande());
        assert!(!e.retour_demande());
    }
}
