// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran « Listes d'envies Cardmarket » : les cartes qui manquent, prêtes
//! à coller.
//!
//! # Une page, pas un panneau — 2026-11-06
//!
//! Le panneau latéral d'abord livré étalait des blocs « Liste 1, Liste 2,
//! Liste 3 » : illisible dès qu'un classeur dépasse une centaine de cartes.
//! L'utilisateur a demandé une page à part, **par classeur** — ou, à son
//! choix, **par rareté** dans chaque classeur (RA02 › Secret Rare…) —, avec
//! pour chaque classeur une liste continue, de la moins chère à la plus
//! chère.
//!
//! # Où Cardmarket dira stop
//!
//! Une liste d'envies Cardmarket prend 150 entrées au plus
//! ([`ygo_app::envies::ENTREES_PAR_LISTE`]). La liste n'est donc pas coupée
//! à l'écran, mais un bandeau très visible marque la 150ᵉ entrée : au-dessus,
//! ce que prend la première liste ; en dessous, ce qui va dans la suivante.
//! Chaque tranche a son propre bouton « Copier », qui ne copie que ses
//! lignes.
//!
//! # Copier-coller
//!
//! Les lignes sont montrées dans une zone de texte en lecture seule : la
//! souris sélectionne, `Ctrl+A` puis `Ctrl+C` copient. Les numéros et les
//! prix sont des colonnes à part — ils ne se copient pas avec le texte.
//!
//! # Toujours à jour, jamais enregistré
//!
//! Le calcul se fait à l'ouverture, sur un fil ([`Calcul`]), pour tous les
//! classeurs. Rien n'est écrit, aucune requête réseau ; « ↻ Recalculer »
//! relit les classeurs. Changer de regroupement ne relit rien
//! ([`Envies::regrouper`]). Les textes des colonnes sont mis en cache à
//! chaque changement, jamais calculés à l'image.

use std::time::Duration;

use eframe::egui;
use ygo_app::cardmarket::euros;
use ygo_app::envies::{Entree, Envies, Groupe, LangueDeLaListe, Regroupement, ENTREES_PAR_LISTE};
use ygo_core::paths::Paths;

use crate::valeur::Calcul;

/// Ce que rend le calcul : les listes, `None` sans données Cardmarket, ou
/// l'erreur en clair.
type Resultat = Result<Option<Envies>, String>;

/// Combien de temps « ✔ Copiée » reste affiché, en secondes.
const DUREE_COPIEE: f64 = 4.0;

/// Largeur de la colonne des classeurs.
const LARGEUR_CLASSEURS: f32 = 260.0;

/// Une tranche de [`ENTREES_PAR_LISTE`] entrées, prête à dessiner : les trois
/// colonnes sont des textes entiers, calculés une fois.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tranche {
    /// Le nom de la liste Cardmarket.
    nom: String,
    /// Combien d'entrées elle contient.
    entrees: usize,
    /// Les numéros globaux, un par ligne, alignés à droite.
    numeros: String,
    /// Le texte à coller, un par ligne — sans retour final : une ligne vide
    /// de plus décalerait les colonnes.
    texte: String,
    /// Les prix, un par ligne, alignés à droite.
    prix: String,
    /// La ligne de texte la plus longue : elle fixe la largeur de la zone.
    plus_longue: String,
}

/// Un classeur de la colonne de gauche.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LigneClasseur {
    code: String,
    entrees: usize,
    listes: usize,
    a_verifier: usize,
    illisible: bool,
}

impl LigneClasseur {
    /// Un classeur sans rien à acheter ni à vérifier.
    fn complet(&self) -> bool {
        !self.illisible && self.entrees == 0 && self.a_verifier == 0
    }
}

/// Tout ce que l'écran dessine, calculé une fois par résultat ou par
/// regroupement.
#[derive(Debug)]
struct Donnees {
    envies: Envies,
    classeurs: Vec<LigneClasseur>,
    /// Les tranches de chaque groupe de `envies.groupes`, dans l'ordre.
    tranches: Vec<Vec<Tranche>>,
}

impl Donnees {
    fn nouvelles(envies: Envies) -> Self {
        let classeurs = envies
            .classeurs
            .iter()
            .map(|code| LigneClasseur {
                code: code.clone(),
                entrees: envies.du_classeur(code).map(|g| g.entrees.len()).sum(),
                listes: envies.du_classeur(code).map(Groupe::listes).sum(),
                a_verifier: envies.a_verifier_du_classeur(code).count(),
                illisible: false,
            })
            .chain(envies.illisibles.iter().map(|code| LigneClasseur {
                code: code.clone(),
                entrees: 0,
                listes: 0,
                a_verifier: 0,
                illisible: true,
            }))
            .collect();
        let tranches = envies.groupes.iter().map(tranches_du_groupe).collect();
        Self {
            envies,
            classeurs,
            tranches,
        }
    }

    /// Change de regroupement, sans rien relire.
    fn regrouper(&mut self, regroupement: Regroupement) {
        self.envies.regrouper(regroupement);
        self.tranches = self.envies.groupes.iter().map(tranches_du_groupe).collect();
        for l in &mut self.classeurs {
            if !l.illisible {
                l.entrees = self
                    .envies
                    .du_classeur(&l.code)
                    .map(|g| g.entrees.len())
                    .sum();
                l.listes = self.envies.du_classeur(&l.code).map(Groupe::listes).sum();
                l.a_verifier = self.envies.a_verifier_du_classeur(&l.code).count();
            }
        }
    }
}

/// L'écran des listes d'envies.
pub struct EcranEnvies {
    paths: Paths,
    /// Le classeur d'où l'on vient, où le retour ramène.
    origine: Option<String>,
    /// Langue du site, pour les liens de recherche des cartes à vérifier.
    francais: bool,
    regroupement: Regroupement,
    calcul: Calcul<Resultat>,
    /// L'erreur ou l'absence de données du dernier calcul.
    echec: Option<Resultat>,
    donnees: Option<Donnees>,
    /// Le classeur affiché.
    selection: Option<String>,
    /// La tranche copiée en dernier — (groupe, tranche) —, et quand.
    copiee: Option<((usize, usize), f64)>,
    retour: bool,
}

impl std::fmt::Debug for EcranEnvies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranEnvies")
            .field("origine", &self.origine)
            .field("selection", &self.selection)
            .field("en_cours", &self.calcul.en_cours())
            .finish_non_exhaustive()
    }
}

impl EcranEnvies {
    /// Ouvre l'écran et lance le calcul pour tous les classeurs, sur un fil.
    ///
    /// `classeur` est celui d'où l'on vient : il est choisi d'emblée, et le
    /// retour y ramène. Sans lui, le premier classeur qui a des entrées.
    #[must_use]
    pub fn ouvrir(paths: Paths, classeur: Option<String>) -> Self {
        let francais = ygo_core::config::Config::charger(paths.app_config())
            .langue()
            .code()
            == "FR";
        let mut ecran = Self {
            paths,
            selection: classeur.clone(),
            origine: classeur,
            francais,
            regroupement: Regroupement::default(),
            calcul: Calcul::default(),
            echec: None,
            donnees: None,
            copiee: None,
            retour: false,
        };
        ecran.lancer();
        ecran
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Le classeur d'où l'on vient, s'il y en a un.
    #[must_use]
    pub fn classeur_d_origine(&self) -> Option<&str> {
        self.origine.as_deref()
    }

    /// Lance le calcul de tous les classeurs, sur un fil.
    fn lancer(&mut self) {
        let paths = self.paths.clone();
        let codes = paths.classeurs_existants();
        let regroupement = self.regroupement;
        self.copiee = None;
        self.echec = None;
        self.calcul = Calcul::lancer("envies-cardmarket", move || {
            ygo_app::envies::preparer(&paths, &codes, regroupement).map_err(|e| e.to_string())
        });
    }

    /// Relève le calcul s'il est fini, et prépare ce qu'on dessine.
    fn relever(&mut self, ctx: &egui::Context) {
        if let Some(resultat) = self.calcul.relever() {
            match resultat {
                Ok(Some(envies)) => {
                    let donnees = Donnees::nouvelles(envies);
                    self.selection = choisir(&donnees.classeurs, self.selection.as_deref());
                    self.donnees = Some(donnees);
                }
                autre => {
                    self.donnees = None;
                    self.echec = Some(autre);
                }
            }
        }
        if self.calcul.en_cours() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Le classeur à montrer : celui demandé s'il existe, sinon le premier qui a
/// des entrées, sinon le premier.
fn choisir(classeurs: &[LigneClasseur], voulu: Option<&str>) -> Option<String> {
    voulu
        .and_then(|v| classeurs.iter().find(|c| c.code == v))
        .or_else(|| classeurs.iter().find(|c| c.entrees > 0))
        .or_else(|| classeurs.first())
        .map(|c| c.code.clone())
}

impl eframe::App for EcranEnvies {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dessiner(racine);
    }
}

impl EcranEnvies {
    /// Relève le calcul puis dessine l'écran entier.
    fn dessiner(&mut self, racine: &mut egui::Ui) {
        self.relever(racine.ctx());
        // Les données sont sorties le temps de l'image : les dessiner demande
        // `&mut self` (boutons, sélection).
        let mut donnees = self.donnees.take();
        self.barre_du_haut(racine, donnees.as_mut());
        if let Some(d) = &donnees {
            self.colonne_des_classeurs(racine, d);
        }
        egui::CentralPanel::default().show(racine, |ui| {
            if self.calcul.en_cours() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Calcul des cartes manquantes…");
                });
                return;
            }
            match (&donnees, &self.echec) {
                (Some(d), _) => self.corps(ui, d),
                (None, Some(Err(e))) => {
                    ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                }
                (None, _) => {
                    ui.label(
                        "Pas de données Cardmarket : les fichiers publics se téléchargent au \
                         démarrage de l'application.",
                    );
                }
            }
        });
        if self.donnees.is_none() {
            self.donnees = donnees;
        }
    }
}

impl EcranEnvies {
    /// L'en-tête : retour, titre, regroupement, recalcul, résumé.
    fn barre_du_haut(&mut self, racine: &mut egui::Ui, mut donnees: Option<&mut Donnees>) {
        egui::Panel::top("entete-envies").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("← Retour").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Listes d'envies Cardmarket");
                ui.add_space(12.0);
                let mut choisi = self.regroupement;
                for r in [Regroupement::ParClasseur, Regroupement::ParRarete] {
                    ui.selectable_value(&mut choisi, r, r.libelle());
                }
                if choisi != self.regroupement {
                    self.regroupement = choisi;
                    self.copiee = None;
                    if let Some(d) = donnees.as_deref_mut() {
                        d.regrouper(choisi);
                    }
                }
                ui.separator();
                let actif = !self.calcul.en_cours();
                if ui
                    .add_enabled(actif, egui::Button::new("↻ Recalculer"))
                    .on_hover_text(
                        "Relit les classeurs : après des cartes ajoutées, la liste se met à jour",
                    )
                    .clicked()
                {
                    self.lancer();
                }
            });
            if let Some(d) = donnees.as_deref() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(resume(&d.envies)).strong());
                    if !d.envies.prix_du.is_empty() {
                        ui.small(format!(
                            "· prix Cardmarket du {}",
                            date_du_guide(&d.envies.prix_du)
                        ));
                    }
                });
            }
            ui.add_space(8.0);
        });
    }

    /// Un classeur par ligne.
    fn colonne_des_classeurs(&mut self, racine: &mut egui::Ui, donnees: &Donnees) {
        egui::Panel::left("classeurs-envies")
            .resizable(true)
            .default_size(LARGEUR_CLASSEURS)
            .show(racine, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("envies-classeurs-defilement")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        for l in donnees.classeurs.iter().filter(|l| !l.illisible) {
                            let choisi = self.selection.as_deref() == Some(l.code.as_str());
                            let mut texte = egui::RichText::new(libelle_de_classeur(l));
                            if l.complet() {
                                texte = texte.weak();
                            }
                            if ui.selectable_label(choisi, texte).clicked() {
                                self.selection = Some(l.code.clone());
                            }
                        }
                        for l in donnees.classeurs.iter().filter(|l| l.illisible) {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                format!("{} : classeur illisible, laissé de côté.", l.code),
                            );
                        }
                    });
            });
    }

    /// Le classeur choisi : en-tête, consignes, listes, cartes à vérifier.
    fn corps(&mut self, ui: &mut egui::Ui, donnees: &Donnees) {
        let Some(code) = self.selection.clone() else {
            ui.label("Aucun classeur.");
            return;
        };
        let maintenant = ui.input(|i| i.time);
        egui::ScrollArea::vertical()
            .id_salt("envies-defilement")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let groupes: Vec<(usize, &Groupe)> = donnees
                    .envies
                    .groupes
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.classeur == code)
                    .collect();
                ui.heading(&code);
                let seuls: Vec<&Groupe> = groupes.iter().map(|(_, g)| *g).collect();
                ui.label(resume_du_classeur(&seuls));
                ui.add_space(4.0);
                if groupes.is_empty() {
                    ui.label("Rien ne manque à ce classeur.");
                } else {
                    egui::CollapsingHeader::new("Sur Cardmarket, ensuite")
                        .id_salt("envies-consignes")
                        .default_open(false)
                        .show(ui, |ui| {
                            let langues: Vec<LangueDeLaListe> = {
                                let mut l: Vec<_> = seuls.iter().map(|g| g.langue).collect();
                                l.dedup();
                                l
                            };
                            for ligne in consignes(&langues) {
                                ui.small(ligne);
                            }
                        });
                }
                for (indice, groupe) in groupes {
                    ui.add_space(10.0);
                    if let Some(tranches) = donnees.tranches.get(indice) {
                        self.groupe(ui, indice, groupe, tranches, maintenant);
                    }
                }
                self.a_verifier(ui, donnees, &code);
            });
    }

    /// Un groupe : son titre, puis ses tranches séparées par les bandeaux.
    fn groupe(
        &mut self,
        ui: &mut egui::Ui,
        indice: usize,
        groupe: &Groupe,
        tranches: &[Tranche],
        maintenant: f64,
    ) {
        ui.label(egui::RichText::new(&groupe.nom).heading());
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(titre_de_langue(groupe.langue)).strong());
            ui.small(detail_de_groupe(groupe));
        });
        ui.add_space(4.0);
        for (i, t) in tranches.iter().enumerate() {
            if i > 0 {
                bandeau_de_coupure(ui, &t.nom);
            }
            self.entete_de_tranche(ui, groupe, (indice, i), t, maintenant);
            lignes_de_tranche(ui, (indice, i), t);
        }
        if let Some(derniere) = tranches.last() {
            ui.small(place_restante(derniere.entrees));
        }
    }

    /// La barre d'une tranche : son nom, sa jauge, ses boutons.
    fn entete_de_tranche(
        &mut self,
        ui: &mut egui::Ui,
        groupe: &Groupe,
        cle: (usize, usize),
        t: &Tranche,
        maintenant: f64,
    ) {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Liste Cardmarket « {} »", t.nom)).strong(),
                    );
                    ui.add(
                        egui::ProgressBar::new(t.entrees as f32 / ENTREES_PAR_LISTE as f32)
                            .desired_width(140.0)
                            .text(format!("{} / {ENTREES_PAR_LISTE}", t.entrees)),
                    );
                    if ui
                        .button(format!("📋 Copier ces {} lignes", t.entrees))
                        .on_hover_text(
                            "À coller sur Cardmarket : Mes envies → la liste → « Ajouter une \
                             decklist »",
                        )
                        .clicked()
                    {
                        ui.ctx().copy_text(groupe.texte_de_tranche(cle.1));
                        self.copiee = Some((cle, maintenant));
                    }
                    if ui
                        .small_button("Copier le nom")
                        .on_hover_text(format!("« {} » : le nom à donner à la liste", t.nom))
                        .clicked()
                    {
                        ui.ctx().copy_text(t.nom.clone());
                    }
                    if let Some((copiee, quand)) = self.copiee {
                        if copiee == cle && maintenant - quand < DUREE_COPIEE {
                            ui.label("✔ Copiée");
                            ui.ctx().request_repaint_after(Duration::from_millis(250));
                        }
                    }
                });
            });
    }

    /// Ce qui manque au classeur sans pouvoir s'écrire.
    fn a_verifier(&self, ui: &mut egui::Ui, donnees: &Donnees, code: &str) {
        let a_verifier: Vec<_> = donnees.envies.a_verifier_du_classeur(code).collect();
        if a_verifier.is_empty() {
            return;
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(format!("À vérifier ({})", a_verifier.len()))
            .id_salt("envies-a-verifier")
            .default_open(false)
            .show(ui, |ui| {
                ui.small(
                    "Ces cartes manquent, mais leur version Cardmarket n'est pas sûre : \
                     elles ne sont dans aucune liste. Ajoutez-les à la main.",
                );
                let langue = ygo_core::cardmarket::langue_du_site(self.francais);
                for a in a_verifier {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("Chercher")
                            .on_hover_text("Ouvrir la recherche de la carte sur Cardmarket")
                            .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(a.offre.url(langue)));
                        }
                        ui.small(format!(
                            "{} · {} · {}",
                            a.origine.set_code, a.origine.version, a.nom
                        ))
                        .on_hover_text(a.motif.explication());
                    });
                }
            });
    }
}

/// Le bandeau qui dit où Cardmarket s'arrête.
fn bandeau_de_coupure(ui: &mut egui::Ui, suivante: &str) {
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(170, 60, 20))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(texte_du_bandeau(suivante))
                    .strong()
                    .color(egui::Color32::WHITE),
            );
        });
    ui.add_space(4.0);
}

/// Les lignes d'une tranche, en trois colonnes alignées : numéros, texte à
/// coller (lecture seule, sélectionnable), prix.
///
/// Les trois colonnes ont la même police, des marges nulles et aucun retour
/// à la ligne : les lignes se répondent une à une. Une barre horizontale
/// couvre les noms trop longs pour la fenêtre.
fn lignes_de_tranche(ui: &mut egui::Ui, cle: (usize, usize), t: &Tranche) {
    let police = egui::TextStyle::Monospace.resolve(ui.style());
    let largeur = ui
        .painter()
        .layout_no_wrap(t.plus_longue.clone(), police, egui::Color32::WHITE)
        .size()
        .x
        + 12.0;
    egui::ScrollArea::horizontal()
        .id_salt(("envies-lignes", cle))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add(
                    egui::Label::new(egui::RichText::new(t.numeros.as_str()).monospace().weak())
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                let mut texte = t.texte.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut texte)
                        .id_salt(("envies-texte", cle))
                        .font(egui::TextStyle::Monospace)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_rows(1)
                        .desired_width(largeur),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(t.prix.as_str()).monospace())
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
        });
}

// ─────────────────────────────────────────────────────────────────────────────
// Les textes — fonctions pures
// ─────────────────────────────────────────────────────────────────────────────

/// « 1 entrée », « 3 entrées ».
fn compte(n: usize, un: &str, plusieurs: &str) -> String {
    if n > 1 {
        format!("{n} {plusieurs}")
    } else {
        format!("{n} {un}")
    }
}

/// La ligne de tête : ce qui s'écrit, en combien de listes, ce qui reste à
/// voir.
///
/// ```
/// use ygo_app::envies::Envies;
/// use ygo_ui::envies::resume;
/// assert_eq!(resume(&Envies::default()), "Rien ne manque.");
/// ```
#[must_use]
pub fn resume(envies: &Envies) -> String {
    if envies.manquantes() == 0 {
        return "Rien ne manque.".to_owned();
    }
    let mut texte = format!(
        "{} · {}",
        compte(envies.entrees(), "entrée", "entrées"),
        compte(envies.listes(), "liste Cardmarket", "listes Cardmarket"),
    );
    if !envies.a_verifier.is_empty() {
        texte.push_str(&format!(" · {} à vérifier", envies.a_verifier.len()));
    }
    texte
}

/// L'en-tête d'un classeur : entrées, cartes, valeur, listes et limite.
#[must_use]
pub fn resume_du_classeur(groupes: &[&Groupe]) -> String {
    let entrees: usize = groupes.iter().map(|g| g.entrees.len()).sum();
    let cartes: u32 = groupes.iter().map(|g| g.cartes()).sum();
    let valeur: u64 = groupes.iter().map(|g| g.valeur()).sum();
    let listes: usize = groupes.iter().map(|g| g.listes()).sum();
    format!(
        "{} · {} · ≈ {} · {} — {ENTREES_PAR_LISTE} entrées max par liste",
        compte(entrees, "entrée", "entrées"),
        compte(cartes as usize, "carte", "cartes"),
        euros(valeur),
        compte(listes, "liste Cardmarket", "listes Cardmarket"),
    )
}

/// La ligne d'un classeur dans la colonne de gauche.
fn libelle_de_classeur(l: &LigneClasseur) -> String {
    let mut texte = format!(
        "{}\n{} · {}",
        l.code,
        compte(l.entrees, "entrée", "entrées"),
        compte(l.listes, "liste", "listes"),
    );
    if l.a_verifier > 0 {
        texte.push_str(&format!("   ⚠ {} à vérifier", l.a_verifier));
    }
    texte
}

/// Ce qu'un groupe contient : prix extrêmes, valeur, entrées sans prix.
#[must_use]
pub fn detail_de_groupe(groupe: &Groupe) -> String {
    let mut texte = compte(groupe.entrees.len(), "entrée", "entrées");
    if let Some((bas, haut)) = groupe.extremes() {
        texte.push_str(&format!(
            " · de {} à {} · ≈ {}",
            euros(bas.into()),
            euros(haut.into()),
            euros(groupe.valeur())
        ));
    }
    match groupe.sans_prix() {
        0 => {}
        1 => texte.push_str(" · 1 sans prix"),
        n => texte.push_str(&format!(" · {n} sans prix")),
    }
    texte
}

/// La langue à régler sur Cardmarket pour un groupe.
#[must_use]
pub fn titre_de_langue(langue: LangueDeLaListe) -> String {
    match langue {
        LangueDeLaListe::Indifferente => {
            "Langue à régler : Indifférent (cartes asiatiques ou toutes langues)".to_owned()
        }
        LangueDeLaListe::Langue(_) => format!("Langue à régler : {}", langue.libelle()),
    }
}

/// Le bandeau placé là où Cardmarket s'arrête.
#[must_use]
pub fn texte_du_bandeau(suivante: &str) -> String {
    format!(
        "⛔ Cardmarket s'arrête ici : {ENTREES_PAR_LISTE} entrées par liste. La suite va dans \
         une nouvelle liste « {suivante} »."
    )
}

/// La place qui reste dans la dernière liste.
#[must_use]
pub fn place_restante(entrees: usize) -> String {
    format!(
        "Place restante dans cette liste : {}",
        ENTREES_PAR_LISTE.saturating_sub(entrees)
    )
}

/// Le prix d'une entrée : « ≈ » s'il n'est pas sûr, « — » sans prix.
#[must_use]
pub fn prix_affiche(entree: &Entree) -> String {
    match entree.prix {
        None => "—".to_owned(),
        Some(p) if entree.prix_sur => euros(p.into()),
        Some(p) => format!("≈ {}", euros(p.into())),
    }
}

/// Les colonnes d'une tranche. `debut` : combien d'entrées la précèdent dans
/// le groupe, pour des numéros qui continuent d'une tranche à l'autre.
fn tranche(nom: String, entrees: &[Entree], debut: usize) -> Tranche {
    let largeur_numero = (debut + entrees.len()).to_string().len();
    let prix: Vec<String> = entrees.iter().map(prix_affiche).collect();
    let largeur_prix = prix.iter().map(|p| p.chars().count()).max().unwrap_or(0);
    let lignes: Vec<String> = entrees.iter().map(Entree::ligne).collect();
    Tranche {
        nom,
        entrees: entrees.len(),
        numeros: (0..entrees.len())
            .map(|i| format!("{:>largeur_numero$}", debut + i + 1))
            .collect::<Vec<_>>()
            .join("\n"),
        prix: prix
            .iter()
            .map(|p| format!("{p:>largeur_prix$}"))
            .collect::<Vec<_>>()
            .join("\n"),
        plus_longue: lignes
            .iter()
            .max_by_key(|l| l.chars().count())
            .cloned()
            .unwrap_or_default(),
        texte: lignes.join("\n"),
    }
}

/// Les tranches d'un groupe.
fn tranches_du_groupe(groupe: &Groupe) -> Vec<Tranche> {
    groupe
        .tranches()
        .enumerate()
        .map(|(i, entrees)| tranche(groupe.nom_de_tranche(i), entrees, i * ENTREES_PAR_LISTE))
        .collect()
}

/// Ce qu'il reste à faire sur Cardmarket, une fois la liste copiée.
#[must_use]
pub fn consignes(langues: &[LangueDeLaListe]) -> Vec<String> {
    let langue = match langues {
        [seule] => format!("« Langues » : {}", seule.libelle()),
        _ => "« Langues » : celle indiquée au-dessus de chaque liste".to_owned(),
    };
    vec![
        "1. Mes envies : créez une liste — « Copier le nom » donne celui proposé.".to_owned(),
        "2. Collez la liste dans « Ajouter une decklist », puis ajoutez.".to_owned(),
        format!(
            "3. Modification en masse : cochez les cartes, choisissez l'état minimum et {langue}, \
             puis « Modify Selected »."
        ),
        "4. Lancez le Shopping Wizard, ou voyez les vendeurs avec le plus de cartes.".to_owned(),
        format!(
            "Une liste prend {ENTREES_PAR_LISTE} cartes au plus. Les cartes achetées, une fois \
             ajoutées à leur classeur, sortent d'elles-mêmes des listes suivantes."
        ),
    ]
}

/// La date du guide des prix, sans l'heure : `2026-10-29T02:53:08+0100` →
/// `29/10/2026`. Une date inattendue reste telle quelle.
///
/// ```
/// use ygo_ui::envies::date_du_guide;
/// assert_eq!(date_du_guide("2026-10-29T02:53:08+0100"), "29/10/2026");
/// assert_eq!(date_du_guide("hier"), "hier");
/// ```
#[must_use]
pub fn date_du_guide(brute: &str) -> String {
    let jour = brute.get(..10).unwrap_or(brute);
    let parties: Vec<&str> = jour.split('-').collect();
    match parties.as_slice() {
        [a, m, j] if a.len() == 4 && m.len() == 2 && j.len() == 2 => format!("{j}/{m}/{a}"),
        _ => brute.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use super::*;
    use ygo_core::cardmarket::LangueDesAnnonces;

    const FR: LangueDeLaListe = LangueDeLaListe::Langue(LangueDesAnnonces::Francais);

    fn entree(nom: &str, prix: Option<u32>, sur: bool) -> Entree {
        Entree {
            quantite: 1,
            nom: nom.into(),
            rang: Some(2),
            extension: "Set A".into(),
            prix,
            prix_sur: sur,
            origines: Vec::new(),
        }
    }

    fn groupe(classeur: &str, n: usize) -> Groupe {
        Groupe {
            classeur: classeur.into(),
            rarete: None,
            langue: FR,
            nom: classeur.into(),
            entrees: (0..n)
                .map(|i| entree(&format!("Carte {i}"), Some(50 + i as u32), true))
                .collect(),
        }
    }

    fn envies() -> Envies {
        Envies {
            classeurs: vec!["AAA".into(), "BBB".into()],
            illisibles: vec!["ZZZ".into()],
            groupes: vec![groupe("AAA", 151), groupe("BBB", 0)],
            prix_du: "2026-10-29T02:53:08+0100".into(),
            ..Envies::default()
        }
    }

    #[test]
    fn les_textes_disent_ce_que_la_liste_contient() {
        let g = groupe("AAA", 2);
        assert_eq!(
            detail_de_groupe(&g),
            "2 entrées · de 0,50 € à 0,51 € · ≈ 1,01 €"
        );
        assert_eq!(titre_de_langue(FR), "Langue à régler : Français");
        assert!(titre_de_langue(LangueDeLaListe::Indifferente).contains("Indifférent"));
        assert!(consignes(&[FR])[2].contains("« Langues » : Français"));
        assert!(consignes(&[FR, LangueDeLaListe::Indifferente])[2].contains("chaque liste"));
        assert!(resume_du_classeur(&[&g]).ends_with("150 entrées max par liste"));
        assert_eq!(place_restante(1), "Place restante dans cette liste : 149");
        assert_eq!(
            texte_du_bandeau("AAA 2/2"),
            "⛔ Cardmarket s'arrête ici : 150 entrées par liste. La suite va dans une \
             nouvelle liste « AAA 2/2 »."
        );
        assert_eq!(resume(&Envies::default()), "Rien ne manque.");
    }

    #[test]
    fn le_prix_dit_l_incertitude_et_l_absence() {
        assert_eq!(prix_affiche(&entree("A", Some(50), true)), "0,50 €");
        assert_eq!(prix_affiche(&entree("A", Some(50), false)), "≈ 0,50 €");
        assert_eq!(prix_affiche(&entree("A", None, true)), "—");
    }

    #[test]
    fn les_tranches_numerotent_en_continu_et_alignent_les_colonnes() {
        let g = groupe("AAA", 151);
        let t = tranches_du_groupe(&g);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].nom, "AAA 1/2");
        assert_eq!(t[1].entrees, 1);
        assert_eq!(t[1].numeros, "151");
        assert_eq!(t[0].numeros.lines().next(), Some("  1"));
        for t in &t {
            let n = t.entrees;
            assert_eq!(t.numeros.lines().count(), n);
            assert_eq!(t.texte.lines().count(), n);
            assert_eq!(t.prix.lines().count(), n);
        }
        assert_eq!(
            g.texte_de_tranche(1),
            "1x Carte 150 (V.2) (Set A)\n",
            "la copie garde son retour final"
        );
    }

    #[test]
    fn le_classeur_a_montrer_se_choisit() {
        let d = Donnees::nouvelles(envies());
        assert_eq!(d.classeurs.len(), 3);
        assert_eq!(d.classeurs[0].listes, 2);
        assert!(d.classeurs[1].complet());
        assert_eq!(choisir(&d.classeurs, Some("BBB")).as_deref(), Some("BBB"));
        assert_eq!(choisir(&d.classeurs, Some("X")).as_deref(), Some("AAA"));
        assert_eq!(choisir(&d.classeurs, None).as_deref(), Some("AAA"));
    }

    /// L'écran s'affiche, sans fenêtre, sur des listes déjà calculées.
    #[test]
    fn l_ecran_s_affiche_sur_des_listes_calculees() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dir.path());
        let mut ecran = EcranEnvies::ouvrir(paths, Some("AAA".into()));
        assert_eq!(ecran.classeur_d_origine(), Some("AAA"));
        // Sans données Cardmarket, le calcul rend `None` : on attend sa fin.
        let ctx = egui::Context::default();
        while ecran.calcul.en_cours() {
            ecran.relever(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        ecran.donnees = Some(Donnees::nouvelles(envies()));
        ecran.selection = Some("AAA".into());
        for _ in 0..3 {
            let entree = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            };
            let mut sortie = ctx.run_ui(entree, |racine| {
                ecran.dessiner(racine);
            });
            sortie.textures_delta.clear();
        }
        assert!(ecran.donnees.is_some());
        assert!(!ecran.retour_demande());
    }
}
