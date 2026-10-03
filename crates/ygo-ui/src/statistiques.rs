// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran des statistiques : où en est la collection.
//!
//! Toute la matière vient de [`ygo_app::statistiques`], dont les chiffres
//! sont figés contre ceux du Python sur les huit classeurs réels. Ici, il
//! ne reste que les pixels — et deux règles pures, [`teinte`] et
//! [`ligne_de_reste`].
//!
//! # Ce que l'écran montre en plus du Python
//!
//! Le Python montrait la décomposition par rareté **classeur par
//! classeur**, jamais l'ensemble. Cinq Secret Rare manquantes réparties
//! sur cinq classeurs n'apparaissaient donc nulle part : chaque classeur
//! en signalait une, et personne ne les additionnait. L'onglet « Toutes
//! raretés confondues » répond à la question qu'on se pose vraiment devant
//! une collection — *qu'est-ce qui me manque le plus ?*

use eframe::egui;

use ygo_app::statistiques::{self, Classeur, ParRarete};
use ygo_core::paths::Paths;

/// Le chevron d'un classeur replié, et celui d'un classeur déplié.
///
/// # La couverture d'une police ne suffit pas : il faut la bonne famille
///
/// `▸` (U+25B8) et `▾` (U+25BE) **existent** dans les polices d'egui — mais
/// seulement dans **Hack**, qui n'est que la police *monospace*. Un bouton
/// ou un libellé emploie la famille *proportionnelle*, composée d'Ubuntu
/// Light, Noto Emoji et emoji-icon-font ; aucune des trois n'a ces deux
/// caractères, et l'écran affichait donc neuf carrés vides.
///
/// `⏵` (U+23F5) et `⏷` (U+23F7) sont dans emoji-icon-font, donc dans la
/// famille proportionnelle. Vérifié avec `fontTools` sur les trois polices
/// de cette famille, pas sur les quatre du dossier.
pub const CHEVRON_REPLIE: &str = "⏵";
/// Le chevron d'un classeur déplié — voir [`CHEVRON_REPLIE`].
pub const CHEVRON_DEPLIE: &str = "⏷";

/// Largeur de la colonne des codes de classeur.
const LARGEUR_NOM: f32 = 92.0;
/// Largeur de la jauge.
const LARGEUR_JAUGE: f32 = 260.0;
/// Largeur de la colonne du pourcentage.
const LARGEUR_PART: f32 = 56.0;
/// Largeur de la colonne « possédées / total ».
const LARGEUR_COMPTEURS: f32 = 96.0;
/// Largeur d'une ligne de contenu.
///
/// Bornée, et non étalée sur toute la fenêtre : sur un écran large, un
/// « il en manque 546 » collé au bord droit se retrouve à huit cents pixels
/// du chiffre qu'il commente, et l'œil ne fait plus le lien.
const LARGEUR_LIGNE: f32 = 820.0;

/// Une cellule de largeur fixe, alignée à **gauche**.
///
/// `add_sized` centre son contenu, ce qui donne une colonne de libellés en
/// dents de scie : « Rare » au milieu, « Quarter Century Secret Rare » d'un
/// bord à l'autre, et aucun début de mot aligné sur le précédent.
fn cellule_gauche(ui: &mut egui::Ui, largeur: f32, texte: egui::RichText) {
    ui.allocate_ui_with_layout(
        egui::vec2(largeur, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // `allocate_ui_with_layout` réserve un rectangle mais le parent
            // n'avance que de ce que l'enfant a **réellement** occupé : sans
            // ce minimum, un libellé court laissait la colonne suivante
            // remonter, et les jauges recommençaient à se décaler d'une
            // ligne à l'autre.
            ui.set_min_width(largeur);
            ui.add(egui::Label::new(texte).truncate());
        },
    );
}

/// Une cellule de largeur fixe, alignée à droite.
///
/// C'est ce qui fait qu'une colonne de nombres se lit : les unités sous les
/// unités. `add_sized` centre, ce qui ne convient qu'au texte.
fn cellule_droite(ui: &mut egui::Ui, largeur: f32, texte: egui::RichText) {
    ui.allocate_ui_with_layout(
        egui::vec2(largeur, 18.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            // Pas de `set_min_width` ici, contrairement à la cellule
            // gauche : une disposition droite-à-gauche s'ancre déjà sur le
            // bord droit du rectangle réservé, donc le parent avance bien
            // de `largeur`. La ligne y avait été copiée par symétrie ; une
            // mutation y a survécu, ce qui est la définition du code mort.
            ui.label(texte);
        },
    );
}

/// La couleur d'une jauge, du rouge au vert selon l'avancement.
///
/// Une jauge d'une seule couleur oblige à lire le pourcentage pour savoir
/// si l'on est loin ou près ; la teinte le dit d'un coup d'œil sur vingt
/// lignes à la fois.
///
/// Les trois composantes sont choisies pour rester lisibles sur fond clair
/// **et** sur fond sombre : pas de jaune pur, pas de vert fluo.
#[must_use]
pub fn teinte(pourcentage: f64) -> egui::Color32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let p = (pourcentage.clamp(0.0, 100.0) / 100.0) as f32;
    // Rouge sombre → ambre → vert forêt.
    let (r, v) = if p < 0.5 {
        (200.0, 60.0 + p * 2.0 * 120.0)
    } else {
        (200.0 - (p - 0.5) * 2.0 * 130.0, 180.0)
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    egui::Color32::from_rgb(r as u8, v as u8, 70)
}

/// La phrase qui dit ce qui manque, ou rien s'il ne manque rien.
///
/// « il en manque 0 » sur un classeur complet est du bruit : la jauge
/// pleine et le « 100 % » le disent déjà deux fois.
///
/// ```
/// use ygo_ui::statistiques::ligne_de_reste;
/// assert_eq!(ligne_de_reste(0), "");
/// assert_eq!(ligne_de_reste(1), "il en manque 1");
/// assert_eq!(ligne_de_reste(546), "il en manque 546");
/// ```
#[must_use]
pub fn ligne_de_reste(manquantes: usize) -> String {
    if manquantes == 0 {
        String::new()
    } else {
        format!("il en manque {manquantes}")
    }
}

/// Ce que l'écran montre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Vue {
    /// Un classeur par ligne.
    #[default]
    ParClasseur,
    /// Les raretés de toute la collection, additionnées.
    ParRarete,
}

/// L'écran des statistiques.
pub struct EcranStatistiques {
    paths: Paths,
    classeurs: Vec<Classeur>,
    cumul: Vec<ParRarete>,
    filtre: String,
    vue: Vue,
    /// Les classeurs dont le détail par rareté est déplié.
    deplies: std::collections::BTreeSet<String>,
    retour: bool,
    duree_lecture: std::time::Duration,
}

impl std::fmt::Debug for EcranStatistiques {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranStatistiques")
            .field("classeurs", &self.classeurs.len())
            .finish_non_exhaustive()
    }
}

impl EcranStatistiques {
    /// Lit l'installation et prépare l'écran.
    #[must_use]
    pub fn ouvrir(paths: Paths) -> Self {
        let debut = std::time::Instant::now();
        let classeurs = statistiques::lister(&paths);
        let duree_lecture = debut.elapsed();
        let cumul = statistiques::raretes_cumulees(&classeurs);
        Self {
            paths,
            classeurs,
            cumul,
            filtre: String::new(),
            vue: Vue::default(),
            deplies: std::collections::BTreeSet::new(),
            retour: false,
            duree_lecture,
        }
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Relit l'installation.
    pub fn relire(&mut self) {
        let debut = std::time::Instant::now();
        self.classeurs = statistiques::lister(&self.paths);
        self.duree_lecture = debut.elapsed();
        self.cumul = statistiques::raretes_cumulees(&self.classeurs);
    }
}

impl eframe::App for EcranStatistiques {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.barre_du_haut(racine);
        self.barre_du_bas(racine);
        egui::CentralPanel::default().show(racine, |ui| {
            if self.classeurs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    ui.heading("Aucun classeur dans cette installation.");
                });
                return;
            }
            match self.vue {
                Vue::ParClasseur => self.liste_des_classeurs(ui),
                Vue::ParRarete => self.liste_des_raretes(ui),
            }
        });
    }
}

impl EcranStatistiques {
    /// L'en-tête : titre, récapitulatif, puis les commandes.
    ///
    /// # Les commandes ont leur propre ligne
    ///
    /// Elles partageaient d'abord la ligne du titre, alignées à droite.
    /// Sur une fenêtre étroite, elles se retrouvaient poussées hors du
    /// cadre — invisibles, sans que rien ne le laisse deviner. Une ligne à
    /// elles ne dépend plus de la largeur restante.
    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-stats").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Mes classeurs").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Statistiques");
            });
            ui.add_space(8.0);
            self.recapitulatif(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.vue, Vue::ParClasseur, "Par classeur");
                ui.selectable_value(&mut self.vue, Vue::ParRarete, "Par rareté");
                ui.separator();
                if self.vue == Vue::ParClasseur {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filtre)
                            .hint_text("filtrer…")
                            .desired_width(180.0),
                    );
                    if !self.filtre.trim().is_empty() && ui.button("✖").clicked() {
                        self.filtre.clear();
                    }
                    ui.separator();
                }
                if ui.button("Recalculer").clicked() {
                    self.relire();
                }
            });
            ui.add_space(8.0);
        });
    }

    /// Les quatre chiffres du haut.
    ///
    /// Ils portent sur **toute** la collection, jamais sur le filtre : un
    /// récapitulatif qui bouge quand on tape dans une case de recherche ne
    /// récapitule plus rien. C'est la règle du Python, et elle est bonne.
    fn recapitulatif(&self, ui: &mut egui::Ui) {
        let t = statistiques::totaux(&self.classeurs);
        ui.horizontal(|ui| {
            for (valeur, libelle) in [
                (format!("{}", t.classeurs), "classeurs"),
                (format!("{}", t.total), "cartes"),
                (format!("{}", t.possedees), "possédées"),
                (format!("{:.0} %", t.pourcentage()), "de la collection"),
                (format!("{}", t.complets), "classeurs complets"),
            ] {
                egui::Frame::group(ui.style())
                    .corner_radius(6.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(valeur).heading().strong());
                            ui.small(egui::RichText::new(libelle).weak());
                        });
                    });
                ui.add_space(6.0);
            }
        });
    }

    fn barre_du_bas(&mut self, racine: &mut egui::Ui) {
        egui::Panel::bottom("mesures-stats").show(racine, |ui| {
            ui.add_space(4.0);
            ui.small(format!(
                "{} classeur(s) lus en {:?}",
                self.classeurs.len(),
                self.duree_lecture
            ));
            ui.add_space(4.0);
        });
    }

    fn liste_des_classeurs(&mut self, ui: &mut egui::Ui) {
        let visibles: Vec<Classeur> = statistiques::filtrer(&self.classeurs, &self.filtre)
            .into_iter()
            .cloned()
            .collect();
        if visibles.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(96.0);
                ui.heading(format!(
                    "Aucun classeur ne correspond à « {} ».",
                    self.filtre.trim()
                ));
                ui.add_space(8.0);
                if ui.button("Tout montrer").clicked() {
                    self.filtre.clear();
                }
            });
            return;
        }

        let mut a_basculer: Option<String> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            for c in &visibles {
                let deplie = self.deplies.contains(&c.nom);
                egui::Frame::group(ui.style())
                    .corner_radius(6.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(LARGEUR_LIGNE);
                        ui.horizontal(|ui| {
                            // Le chevron seul, de largeur fixe : mettre le
                            // code dans le bouton faisait varier sa largeur
                            // avec la longueur du nom, et décalait toute la
                            // ligne — les jauges ne commençaient pas au même
                            // endroit d'une ligne à l'autre.
                            let chevron = if deplie {
                                CHEVRON_DEPLIE
                            } else {
                                CHEVRON_REPLIE
                            };
                            if ui
                                .add_sized([24.0, 20.0], egui::Button::new(chevron))
                                .on_hover_text("Voir la répartition par rareté")
                                .clicked()
                            {
                                a_basculer = Some(c.nom.clone());
                            }
                            ui.add_sized(
                                [LARGEUR_NOM, 20.0],
                                egui::Label::new(
                                    egui::RichText::new(c.nom.as_str()).strong().monospace(),
                                ),
                            );
                            jauge(ui, c.pourcentage(), LARGEUR_JAUGE);
                            cellule_droite(
                                ui,
                                LARGEUR_PART,
                                egui::RichText::new(format!("{:.0} %", c.pourcentage())),
                            );
                            cellule_droite(
                                ui,
                                LARGEUR_COMPTEURS,
                                egui::RichText::new(format!("{} / {}", c.possedees, c.total))
                                    .monospace(),
                            );
                            ui.add_space(10.0);
                            ui.small(egui::RichText::new(ligne_de_reste(c.manquantes())).weak());
                        });
                        if deplie {
                            ui.add_space(6.0);
                            ui.separator();
                            for r in &c.raretes {
                                ligne_rarete(ui, r);
                            }
                        }
                    });
                ui.add_space(4.0);
            }
        });
        if let Some(nom) = a_basculer {
            if !self.deplies.remove(&nom) {
                self.deplies.insert(nom);
            }
        }
    }

    fn liste_des_raretes(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.label("Toutes raretés confondues, tous classeurs additionnés.");
            ui.add_space(8.0);
            for r in &self.cumul {
                egui::Frame::group(ui.style())
                    .corner_radius(6.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(LARGEUR_LIGNE);
                        ligne_rarete(ui, r);
                    });
                ui.add_space(2.0);
            }
        });
    }
}

/// Une ligne de rareté, alignée sur les mêmes colonnes que les classeurs.
fn ligne_rarete(ui: &mut egui::Ui, r: &ParRarete) {
    ui.horizontal(|ui| {
        ui.add_space(24.0); // sous le chevron, pour que tout s'aligne
        cellule_gauche(
            ui,
            LARGEUR_NOM + 130.0,
            egui::RichText::new(r.rarete.as_str()).small(),
        );
        jauge(ui, r.pourcentage(), LARGEUR_JAUGE - 130.0);
        cellule_droite(
            ui,
            LARGEUR_PART,
            egui::RichText::new(format!("{:.0} %", r.pourcentage())).small(),
        );
        cellule_droite(
            ui,
            LARGEUR_COMPTEURS,
            egui::RichText::new(format!("{} / {}", r.possedees, r.total))
                .small()
                .monospace(),
        );
        ui.add_space(10.0);
        ui.small(egui::RichText::new(ligne_de_reste(r.manquantes())).weak());
    });
}

/// Une jauge colorée selon son remplissage.
///
/// # Le pourcentage est écrit à côté, pas dedans
///
/// egui aligne le texte d'une `ProgressBar` à gauche de la barre entière.
/// À 6 %, « 6,2 % » se dessinait donc à cheval sur le peu de rouge rempli
/// et sur le fond sombre — illisible sur les deux tiers de sa longueur.
/// Dans sa propre colonne, il se lit toujours, et les pourcentages
/// s'alignent verticalement d'une ligne à l'autre.
fn jauge(ui: &mut egui::Ui, pourcentage: f64, largeur: f32) {
    #[allow(clippy::cast_possible_truncation)]
    let part = (pourcentage / 100.0) as f32;
    ui.add_sized(
        [largeur, 16.0],
        egui::ProgressBar::new(part)
            .fill(teinte(pourcentage))
            .corner_radius(4.0),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Une cellule occupe sa largeur même quand son texte est court.
    ///
    /// # Le défaut que ce test attrape
    ///
    /// `allocate_ui_with_layout` réserve un rectangle, mais le parent
    /// n'avance ensuite que de ce que l'enfant a **réellement** occupé.
    /// Sans le `set_min_width`, une cellule contenant « M » avançait de dix
    /// pixels au lieu de deux cents, la colonne suivante remontait, et les
    /// jauges se décalaient d'une ligne à l'autre — exactement ce que la
    /// capture d'écran montrait. Ici on mesure l'avancée du curseur, pas
    /// l'intention du code.
    #[test]
    fn une_cellule_occupe_sa_largeur_meme_avec_un_texte_court() {
        assert_eq!(
            encombrement(200.0, "M"),
            (208.0, 18.0),
            "200 px de cellule plus 8 px d'espacement, sur 18 px de haut"
        );
    }

    /// La cellule droite occupe sa largeur, elle aussi — sans `set_min_width`.
    ///
    /// C'est la contrepartie de la ligne **absente** dans `cellule_droite` :
    /// une disposition droite-à-gauche s'ancre sur le bord droit du
    /// rectangle réservé, donc le parent avance bien de `largeur` sans
    /// qu'on ait à l'exiger. Ce test fige ce raisonnement : si la
    /// disposition redevenait gauche-à-droite, l'avancée tomberait à la
    /// largeur du texte et la colonne des compteurs se disloquerait.
    #[test]
    fn la_cellule_droite_occupe_sa_largeur_sans_minimum() {
        let mut mesure = (0.0_f32, 0.0_f32);
        avec_polices(|ui| {
            ui.horizontal(|ui| {
                let depart = ui.cursor().min.x;
                cellule_droite(ui, 96.0, egui::RichText::new("35 / 35"));
                mesure = (ui.cursor().min.x - depart, ui.min_rect().height());
            });
        });
        assert_eq!(
            mesure,
            (104.0, 18.0),
            "96 px de cellule plus 8 px d'espacement, sur 18 px de haut"
        );
    }

    /// Un libellé trop long est coupé : il ne déforme pas sa ligne.
    ///
    /// Deux déformations, deux mesures. « Quarter Century Secret Rare » fait
    /// 165 px : dans une colonne de 40, **sans rien**, il l'élargit d'autant
    /// et la jauge de cette ligne-là part cent pixels plus loin que les
    /// autres (vérifié à l'écran, deux captures de la vue par rareté avec la
    /// colonne rétrécie). **Avec `wrap`**, la largeur tient mais le texte
    /// passe à la ligne et c'est la hauteur qui triple. Seul `truncate`
    /// laisse la ligne intacte, alors on mesure les deux.
    #[test]
    fn un_libelle_trop_long_ne_deforme_pas_sa_ligne() {
        assert_eq!(
            encombrement(40.0, "Quarter Century Secret Rare"),
            encombrement(40.0, "Rare"),
            "le libellé long a changé l'encombrement de sa cellule"
        );
    }

    /// La place que prend une cellule : ce dont le curseur avance, et la
    /// hauteur de la ligne qui la contient.
    fn encombrement(largeur: f32, texte: &str) -> (f32, f32) {
        let (mut large, mut haut) = (0.0_f32, 0.0_f32);
        avec_polices(|ui| {
            ui.horizontal(|ui| {
                let depart = ui.cursor().min.x;
                cellule_gauche(ui, largeur, egui::RichText::new(texte));
                large = ui.cursor().min.x - depart;
                haut = ui.min_rect().height();
            });
        });
        (large, haut)
    }

    /// Un `Ui` de test **avec ses polices**.
    ///
    /// # Pourquoi pas `egui::__run_test_ui`
    ///
    /// Parce qu'il fait `set_fonts(FontDefinitions::empty())` pour aller
    /// vite. Sans police, tout texte mesure la même chose — zéro — et un
    /// test de mise en page qui dépend de la longueur du texte passe quoi
    /// qu'on lui donne. C'est ce qui m'a fait écrire deux tests qui ne
    /// prouvaient rien : ils voyaient « Rare » et « Quarter Century Secret
    /// Rare » exactement de la même largeur.
    fn avec_polices(mut contenu: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let entree = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        // Deux passes : la première charge les polices, la seconde mesure.
        for _ in 0..2 {
            ctx.run_ui(entree.clone(), |ui| contenu(ui))
                .drop_without_applying_deltas();
        }
    }

    /// La teinte va du rouge au vert, sans jamais sortir des bornes.
    #[test]
    fn la_teinte_va_du_rouge_au_vert() {
        let vide = teinte(0.0);
        let plein = teinte(100.0);
        assert!(vide.r() > vide.g(), "à zéro, ça tire au rouge");
        assert!(plein.g() > plein.r(), "à cent, ça tire au vert");

        // Monotone : le vert ne redescend jamais quand on progresse.
        let mut precedent = 0_u8;
        for p in 0..=100 {
            let c = teinte(f64::from(p));
            assert!(
                c.g() >= precedent,
                "le vert redescend à {p} % : {} après {precedent}",
                c.g()
            );
            precedent = c.g();
        }

        // Le dégradé progresse vraiment, dans les deux moitiés.
        //
        // « monotone » ci-dessus est trop faible pour attraper un dégradé
        // mort : un vert constant dans la première moitié, ou un seuil
        // déplacé qui fait descendre le rouge dès 0 %, passaient tous
        // deux au travers. Deux mutations y survivaient.
        assert!(
            teinte(0.0).g() < teinte(25.0).g() && teinte(25.0).g() < teinte(49.0).g(),
            "le vert monte pour de bon dans la première moitié"
        );
        assert_eq!(
            teinte(0.0).r(),
            teinte(49.0).r(),
            "et le rouge y reste à son maximum : c'est le vert qui travaille"
        );
        assert!(
            teinte(50.0).r() > teinte(75.0).r() && teinte(75.0).r() > teinte(100.0).r(),
            "dans la seconde moitié, c'est le rouge qui se retire"
        );

        // Hors bornes : on ne panique pas, et on ne déborde pas.
        assert_eq!(teinte(-50.0), teinte(0.0));
        assert_eq!(teinte(500.0), teinte(100.0));
    }

    /// Les chevrons doivent rester dans la famille **proportionnelle**.
    ///
    /// Ce test ne lit pas les polices — il fige les deux points de code
    /// vérifiés à la main avec `fontTools`. Son rôle est de faire trébucher
    /// quiconque les remplacerait par un chevron « plus joli » sans
    /// revérifier : `▸` et `▾` existent dans Hack, la police *monospace*, et
    /// nulle part dans la famille des boutons. L'écran affichait neuf carrés
    /// vides.
    #[test]
    fn les_chevrons_sont_ceux_qui_ont_ete_verifies() {
        assert_eq!(CHEVRON_REPLIE, "\u{23F5}", "⏵ — dans emoji-icon-font");
        assert_eq!(CHEVRON_DEPLIE, "\u{23F7}", "⏷ — dans emoji-icon-font");
        assert_ne!(CHEVRON_REPLIE, "▸", "Hack seulement : carré vide");
        assert_ne!(CHEVRON_DEPLIE, "▾", "Hack seulement : carré vide");
        assert_ne!(CHEVRON_REPLIE, CHEVRON_DEPLIE);
    }

    /// « il en manque 0 » est du bruit : la jauge pleine le dit déjà.
    #[test]
    fn un_classeur_complet_ne_dit_pas_ce_qui_lui_manque() {
        assert_eq!(ligne_de_reste(0), "");
        assert_eq!(ligne_de_reste(1), "il en manque 1");
        assert_eq!(ligne_de_reste(546), "il en manque 546");
    }

    fn installation(lignes: &[(&str, &str, i64)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let mut par_classeur: std::collections::BTreeMap<&str, Vec<(&str, i64)>> =
            std::collections::BTreeMap::new();
        for (classeur, rarete, possedee) in lignes {
            par_classeur
                .entry(classeur)
                .or_default()
                .push((rarete, *possedee));
        }
        for (code, cartes) in par_classeur {
            std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
            let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
            conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            for (rarete, possedee) in cartes {
                conn.execute(
                    "INSERT INTO cards (name, rarity, possessed, quantite)
                     VALUES ('C', ?1, ?2, ?2)",
                    ygo_db::rusqlite::params![rarete, possedee],
                )
                .unwrap();
            }
        }
        (tmp, paths)
    }

    /// L'écran lit l'installation à l'ouverture, cumul compris.
    #[test]
    fn l_ecran_lit_tout_a_l_ouverture() {
        let (_tmp, paths) = installation(&[
            ("EGO1", "Common", 1),
            ("EGO1", "Secret Rare", 0),
            ("RA02", "Common", 1),
        ]);
        let ecran = EcranStatistiques::ouvrir(paths);
        assert_eq!(ecran.classeurs.len(), 2);
        assert_eq!(ecran.cumul.len(), 2, "Common et Secret Rare, cumulées");
        assert_eq!(ecran.cumul[0].rarete, "Common");
        assert_eq!(ecran.cumul[0].total, 2, "une dans chaque classeur");
        assert_eq!(ecran.cumul[0].possedees, 2);
        assert_eq!(ecran.cumul[1].manquantes(), 1);
    }

    /// Le dépliage est un interrupteur, et il porte sur un classeur à la
    /// fois.
    #[test]
    fn le_depliage_est_un_interrupteur_par_classeur() {
        let (_tmp, paths) = installation(&[("EGO1", "Common", 1), ("RA02", "Common", 1)]);
        let mut ecran = EcranStatistiques::ouvrir(paths);
        assert!(ecran.deplies.is_empty());

        // La bascule, telle que la liste l'applique.
        let basculer = |e: &mut EcranStatistiques, nom: &str| {
            if !e.deplies.remove(nom) {
                e.deplies.insert(nom.to_owned());
            }
        };
        basculer(&mut ecran, "EGO1");
        assert_eq!(ecran.deplies.len(), 1);
        basculer(&mut ecran, "RA02");
        assert_eq!(ecran.deplies.len(), 2, "les deux peuvent être ouverts");
        basculer(&mut ecran, "EGO1");
        assert_eq!(ecran.deplies.len(), 1);
        assert!(ecran.deplies.contains("RA02"));
    }

    /// Relire prend en compte ce qui a changé en base.
    #[test]
    fn relire_voit_les_nouvelles_possessions() {
        let (_tmp, paths) = installation(&[("EGO1", "Common", 0), ("EGO1", "Common", 0)]);
        let mut ecran = EcranStatistiques::ouvrir(paths.clone());
        assert_eq!(ecran.classeurs[0].possedees, 0);

        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db("EGO1")).unwrap();
        conn.execute("UPDATE cards SET possessed = 1 WHERE rowid = 1", ())
            .unwrap();
        drop(conn);

        ecran.relire();
        assert_eq!(ecran.classeurs[0].possedees, 1);
        assert_eq!(ecran.cumul[0].possedees, 1, "le cumul suit");
    }

    /// Le retour se consomme une seule fois.
    #[test]
    fn le_retour_se_consomme() {
        let (_tmp, paths) = installation(&[("EGO1", "Common", 1)]);
        let mut ecran = EcranStatistiques::ouvrir(paths);
        ecran.retour = true;
        assert!(ecran.retour_demande());
        assert!(!ecran.retour_demande());
    }
}
