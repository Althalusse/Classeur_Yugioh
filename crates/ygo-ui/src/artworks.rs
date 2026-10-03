// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les artworks qu'une carte peut prendre, et celui qu'elle a.
//!
//! Portage de `ui/dialog_anomalies.py`, en écran plutôt qu'en dialogue.
//!
//! # « Anomalie » est le nom du mécanisme, pas celui de l'écran
//!
//! Le scan qui alimente cette liste s'appelle *anomalies* — il constate qu'une
//! source connaît un artwork d'un côté et pas de l'autre. Mais ce que
//! l'utilisateur fait ici n'a rien d'une réparation d'erreur : **il choisit
//! l'illustration de ses cartes**. Le mot « anomalie » n'apparaît nulle part à
//! l'écran. Une carte a des artworks, on en ajoute.
//!
//! # La refonte du 2026-09-21 : on ne voyait pas la différence
//!
//! La première version montrait une ligne par **rareté**, deux vignettes de
//! 72 × 105 points, l'image en place grisée. Retour d'usage : « on ne voit
//! aucune différence entre les arts ». Quatre causes, toutes mesurées :
//!
//! 1. les fichiers sont les images YGOPRODeck **complètes**, 421 × 614 — on
//!    les montrait à 17 % de leur taille ;
//! 2. ce qui diffère — l'illustration — n'occupait qu'une soixantaine de
//!    points de large ;
//! 3. l'image en place était **atténuée** pour qu'on repère la proposition :
//!    on comparait du gris à de la couleur ;
//! 4. la même paire revenait une fois par rareté : **379 lignes pour 205
//!    décisions** sur l'installation réelle, 15 pour 4 sur `LOCR-JP`.
//!
//! L'écran est désormais en deux parts :
//!
//! - **à gauche**, une ligne par numéro ([`ygo_app::anomalies::regrouper`]),
//!   ses raretés en pastilles, une case à trois états ;
//! - **à droite**, une grille : l'image en place et chaque artwork proposé,
//!   **en grand et en couleurs**, puis une ligne par rareté avec une case par
//!   artwork. Survoler une rareté montre **son** image en place : dans 37
//!   numéros sur 205, les raretés d'une carte n'ont pas la même (scans
//!   Yugipedia par rareté).
//!
//! Le cas à plusieurs artworks — 21 numéros, jusqu'à 8 pour Dark Magician —
//! est le cas général de la grille, pas une exception : chaque case se coche
//! seule, deux artworks peuvent l'être pour la même rareté.
//!
//! # Sans l'image, la liste ne veut rien dire
//!
//! Les fichiers sont ceux du classeur, au même nom, dans `img/small` : un
//! aperçu téléchargé ici est l'image définitive, obtenue plus tôt. Ceux qui
//! manquent sont demandés au fil de téléchargement, par lots, **en commençant
//! par le numéro qu'on regarde**.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui;
use ygo_app::anomalies::{self, Anomalie, Correction, Numero};
use ygo_core::config::{Config, SourceImage};
use ygo_core::paths::Paths;
use ygo_images::plan::Cible;

use crate::images::{Cache, Cle};

/// Le rapport largeur / hauteur d'une carte — celui des fichiers YGOPRODeck,
/// 421 × 614.
const RATIO_CARTE: f32 = 421.0 / 614.0;
/// Hauteur des miniatures de la liste.
const HAUTEUR_LISTE: f32 = 64.0;
/// Hauteur des miniatures d'une ligne de rareté.
const HAUTEUR_RARETE: f32 = 46.0;
/// Bornes des grandes images de la comparaison.
const HAUTEUR_GRANDE_MIN: f32 = 150.0;
/// Au-delà, les fichiers de 614 points seraient agrandis — et flous.
const HAUTEUR_GRANDE_MAX: f32 = 460.0;
/// Largeur de la colonne des raretés dans la grille.
const LARGEUR_LIBELLES: f32 = 210.0;
/// Largeur de départ de la liste.
const LARGEUR_LISTE: f32 = 400.0;
/// Combien d'aperçus manquants demander en une fois.
const LOT_APERCUS: usize = 24;
/// Au-delà de ce nombre d'artworks proposés pour un numéro, l'écran prévient
/// que le scan propose tout ce que la carte a connu, pas ce que le set a
/// imprimé.
const SEUIL_AVERTISSEMENT: usize = 3;
/// Le doré des propositions — celui des quantités du classeur.
const COULEUR_OR: egui::Color32 = egui::Color32::from_rgb(212, 175, 55);

/// Ce que l'écran regarde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Portee {
    /// Une carte précise d'un classeur — « Modifier l'artwork ».
    Carte {
        /// Le classeur.
        classeur: String,
        /// Le tirage.
        set_code: String,
    },
    /// Tout un classeur.
    Classeur(String),
    /// Toute l'installation.
    Tout,
}

impl Portee {
    /// Le classeur concerné, s'il y en a un seul.
    #[must_use]
    pub fn classeur(&self) -> Option<&str> {
        match self {
            Self::Carte { classeur, .. } | Self::Classeur(classeur) => Some(classeur),
            Self::Tout => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Règles pures
// ─────────────────────────────────────────────────────────────────────────────

/// Une case de la grille : l'indice de l'artwork dans le numéro, et la rareté.
pub type Case = (usize, String);

/// L'état d'une case à trois états.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coche {
    /// Rien à poser, ou rien de coché.
    Aucune,
    /// Une partie de ce qui reste à poser.
    Partielle,
    /// Tout ce qui reste à poser.
    Toutes,
}

/// D'où vient une image en place, d'après son nom de fichier.
///
/// Les images YGOPRODeck portent l'identifiant numérique de la carte ; les
/// scans Yugipedia, le nom du fichier du wiki. La règle suit celle qui a
/// **fabriqué** ces noms ([`ygo_app::anomalies::fichier_de`]).
///
/// ```
/// use ygo_ui::artworks::origine_image;
/// assert_eq!(origine_image(Some("89631139.jpg")), "image YGOPRODeck");
/// assert_eq!(origine_image(Some("DarkMagician-RA04-EN-PlSR.png")), "scan Yugipedia");
/// assert_eq!(origine_image(None), "pas d'image");
/// ```
#[must_use]
pub fn origine_image(fichier: Option<&str>) -> &'static str {
    let Some(fichier) = fichier else {
        return "pas d'image";
    };
    let tige = Path::new(fichier)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !tige.is_empty() && tige.chars().all(|c| c.is_ascii_digit()) {
        "image YGOPRODeck"
    } else {
        "scan Yugipedia"
    }
}

/// La hauteur des grandes images, pour qu'image en place et propositions
/// tiennent côte à côte et laissent la place aux raretés.
///
/// Bornée en bas pour rester lisible — la grille défile alors de côté — et en
/// haut pour ne jamais agrandir un fichier de 614 points au-delà de ce qu'il
/// porte.
///
/// ```
/// use ygo_ui::artworks::hauteur_des_images;
/// // Un seul artwork proposé, écran large : on monte au plafond.
/// assert_eq!(hauteur_des_images(1500.0, 1000.0, 1, 3), 460.0);
/// // Huit artworks : les neuf colonnes se partagent la largeur.
/// assert!(hauteur_des_images(1500.0, 1000.0, 8, 2) < 250.0);
/// // Jamais sous le plancher, même dans une fenêtre minuscule.
/// assert_eq!(hauteur_des_images(300.0, 200.0, 8, 4), 150.0);
/// ```
#[must_use]
pub fn hauteur_des_images(largeur: f32, hauteur: f32, artworks: usize, raretes: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let colonnes = (artworks + 1) as f32;
    #[allow(clippy::cast_precision_loss)]
    let lignes = raretes as f32;
    let par_colonne = ((largeur - LARGEUR_LIBELLES) / colonnes - 16.0).max(0.0);
    let par_largeur = par_colonne / RATIO_CARTE;
    // En-tête, étiquettes, lignes de raretés, boutons.
    let par_hauteur = hauteur - 150.0 - lignes * (HAUTEUR_RARETE + 10.0);
    par_largeur
        .min(par_hauteur)
        .clamp(HAUTEUR_GRANDE_MIN, HAUTEUR_GRANDE_MAX)
}

/// Un numéro tel que l'écran le tient : la proposition, les images, les
/// cases cochées.
#[derive(Debug, Clone)]
pub struct NumeroUi {
    /// Ce que le scan propose.
    pub numero: Numero,
    /// L'image en place pour chaque rareté.
    pub en_place: HashMap<String, Option<String>>,
    /// Le fichier de chaque artwork proposé, dans l'ordre de
    /// `numero.artworks`.
    pub proposes: Vec<Option<String>>,
    /// Les cases cochées.
    pub coches: BTreeSet<Case>,
    /// Pour chaque artwork proposé : son image est-elle **identique** à une
    /// image déjà en place ? `None` tant qu'un des fichiers manque.
    pub identiques: Vec<Option<bool>>,
}

impl NumeroUi {
    /// La proposition d'une case, si l'artwork est proposé pour cette rareté.
    #[must_use]
    pub fn proposition(&self, case: &Case) -> Option<&Anomalie> {
        self.numero.artworks.get(case.0)?.pour(&case.1)
    }

    /// Les cases qui restent à poser.
    #[must_use]
    pub fn a_poser(&self) -> Vec<Case> {
        let mut cases = Vec::new();
        for (i, p) in self.numero.artworks.iter().enumerate() {
            for a in &p.par_rarete {
                if !a.corrige {
                    cases.push((i, a.missing_set_rarity.clone()));
                }
            }
        }
        cases
    }

    /// Les cases cochées **et** encore à poser.
    #[must_use]
    pub fn cochees(&self) -> Vec<Anomalie> {
        self.coches
            .iter()
            .filter_map(|c| self.proposition(c))
            .filter(|a| !a.corrige)
            .cloned()
            .collect()
    }

    /// L'état de la case à trois états de la liste.
    #[must_use]
    pub fn etat(&self) -> Coche {
        let a_poser = self.a_poser();
        let cochees = a_poser.iter().filter(|c| self.coches.contains(*c)).count();
        match cochees {
            0 => Coche::Aucune,
            n if n == a_poser.len() => Coche::Toutes,
            _ => Coche::Partielle,
        }
    }

    /// L'artwork d'indice `i` est-il la même image que celle en place ?
    #[must_use]
    pub fn identique(&self, i: usize) -> bool {
        self.identiques.get(i).copied().flatten() == Some(true)
    }

    /// Ce qu'un geste « tout » coche : ce qui reste à poser, **sauf** les
    /// images identiques à celles en place — les ajouter mettrait un doublon
    /// dans le classeur. Elles restent cochables une à une.
    #[must_use]
    pub fn a_cocher_en_bloc(&self) -> Vec<Case> {
        self.a_poser()
            .into_iter()
            .filter(|(i, _)| !self.identique(*i))
            .collect()
    }

    /// Tout coché → tout décocher ; sinon, tout cocher. C'est le geste d'une
    /// case à trois états dans l'explorateur.
    pub fn basculer_tout(&mut self) {
        let bloc = self.a_cocher_en_bloc();
        let tout_coche = !bloc.is_empty() && bloc.iter().all(|c| self.coches.contains(c));
        if tout_coche || (bloc.is_empty() && !self.coches.is_empty()) {
            self.coches.clear();
        } else {
            self.coches.extend(bloc);
        }
    }

    /// Compare chaque artwork proposé aux images en place, là où ce n'est
    /// pas encore fait.
    pub fn comparer(&mut self, dossier: &Path) {
        let en_place: BTreeSet<String> = self.en_place.values().flatten().cloned().collect();
        self.identiques.resize(self.proposes.len(), None);
        for (i, propose) in self.proposes.iter().enumerate() {
            if self.identiques.get(i).copied().flatten().is_some() {
                continue;
            }
            let Some(propose) = propose else {
                continue;
            };
            let mut verdict: Option<bool> = None;
            for actuel in &en_place {
                match anomalies::images_identiques(dossier, propose, actuel) {
                    Some(true) => {
                        verdict = Some(true);
                        break;
                    }
                    Some(false) => verdict = Some(false),
                    None => {}
                }
            }
            if let Some(v) = self.identiques.get_mut(i) {
                *v = verdict;
            }
        }
    }

    /// Coche ou décoche une case — seulement si elle existe et reste à poser.
    pub fn basculer(&mut self, case: Case) {
        if self.proposition(&case).is_none_or(|a| a.corrige) {
            return;
        }
        if !self.coches.remove(&case) {
            self.coches.insert(case);
        }
    }

    /// Une rareté a-t-elle au moins une case cochée ?
    #[must_use]
    pub fn rarete_cochee(&self, rarete: &str) -> bool {
        self.coches.iter().any(|(_, r)| r == rarete)
    }

    /// Toutes les propositions de cette rareté sont-elles déjà dans le
    /// classeur ?
    #[must_use]
    pub fn rarete_posee(&self, rarete: &str) -> bool {
        self.numero
            .artworks
            .iter()
            .filter_map(|p| p.pour(rarete))
            .all(|a| a.corrige)
    }
}

/// La clé qui retrouve un numéro d'une lecture à l'autre.
fn cle(n: &Numero) -> (String, String) {
    (n.classeur.clone(), n.set_code.clone())
}

// ─────────────────────────────────────────────────────────────────────────────
// L'écran
// ─────────────────────────────────────────────────────────────────────────────

/// L'écran des artworks.
pub struct EcranArtworks {
    paths: Paths,
    portee: Portee,
    source: SourceImage,
    numeros: Vec<NumeroUi>,
    /// Le numéro montré à droite.
    courant: usize,
    /// La rareté dont l'image en place est montrée — choisie au clic.
    rarete_choisie: Option<String>,
    /// Celle que le pointeur survole, relevée à l'image précédente : elle
    /// l'emporte sur le choix tant que dure le survol.
    rarete_survolee: Option<String>,
    /// La liste doit amener le numéro courant à l'écran — après une
    /// navigation au clavier.
    suivre_courant: bool,
    cache: Cache,
    /// Les aperçus déjà demandés au fil, pour ne pas les redemander à chaque
    /// image.
    demandes: std::collections::HashSet<String>,
    /// Ceux que l'application doit aller chercher.
    a_telecharger: Vec<Cible>,
    message: Option<(String, bool)>,
    retour: bool,
    ecrit: bool,
}

impl std::fmt::Debug for EcranArtworks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranArtworks")
            .field("portee", &self.portee)
            .field("numeros", &self.numeros.len())
            .field("courant", &self.courant)
            .finish_non_exhaustive()
    }
}

impl EcranArtworks {
    /// Ouvre l'écran sur une portée.
    ///
    /// Lit ce qui est **déjà** connu. Le scan reste un geste explicite : il
    /// relit `cardinfo.db` en entier, et l'ouverture doit être immédiate.
    #[must_use]
    pub fn ouvrir(paths: Paths, portee: Portee) -> Self {
        let source = Config::charger(paths.app_config()).source_image();
        let mut ecran = Self {
            paths,
            portee,
            source,
            numeros: Vec::new(),
            courant: 0,
            rarete_choisie: None,
            rarete_survolee: None,
            suivre_courant: false,
            cache: Cache::default(),
            demandes: std::collections::HashSet::new(),
            a_telecharger: Vec::new(),
            message: None,
            retour: false,
            ecrit: false,
        };
        ecran.relire();
        ecran
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Reprend le fait qu'un classeur a été modifié.
    pub fn a_ecrit(&mut self) -> bool {
        std::mem::take(&mut self.ecrit)
    }

    /// La portée regardée.
    #[must_use]
    pub fn portee(&self) -> &Portee {
        &self.portee
    }

    /// Reprend les aperçus à télécharger — l'écran demande, l'application va
    /// chercher.
    pub fn apercus_demandes(&mut self) -> Vec<Cible> {
        std::mem::take(&mut self.a_telecharger)
    }

    /// Une image vient d'arriver : les emplacements vides retentent leur
    /// lecture.
    pub fn rafraichir_images(&mut self) {
        self.cache.oublier_manquants();
        let dossier = self.paths.img_small();
        for n in &mut self.numeros {
            n.comparer(&dossier);
        }
    }

    /// Combien de propositions restent à poser.
    #[must_use]
    pub fn restantes(&self) -> usize {
        self.numeros.iter().map(|n| n.numero.restantes()).sum()
    }

    /// Combien de cases sont cochées, tous numéros confondus.
    fn cochees(&self) -> usize {
        self.numeros.iter().map(|n| n.cochees().len()).sum()
    }

    /// Relit les propositions, en gardant ce qui était coché ailleurs.
    ///
    /// Ajouter la sélection d'un numéro relit tout — sans cette reprise, les
    /// cases cochées sur les autres numéros disparaîtraient à chaque ajout.
    fn relire(&mut self) {
        let propositions = match &self.portee {
            Portee::Carte { classeur, set_code } => {
                anomalies::propositions_pour_carte(&self.paths, classeur, set_code)
                    .unwrap_or_default()
            }
            Portee::Classeur(code) => anomalies::lire(&self.paths, Some(code)).unwrap_or_default(),
            Portee::Tout => anomalies::lire(&self.paths, None).unwrap_or_default(),
        };
        let anciennes: HashMap<(String, String), BTreeSet<Case>> = self
            .numeros
            .drain(..)
            .map(|n| (cle(&n.numero), n.coches))
            .collect();
        let courant = self.numeros.get(self.courant).map(|n| cle(&n.numero));

        // Les illustrations en place, un classeur à la fois : une requête par
        // classeur, pas une par ligne.
        let mut tables: HashMap<String, HashMap<(String, String), String>> = HashMap::new();
        for numero in anomalies::regrouper(propositions) {
            let table = tables.entry(numero.classeur.clone()).or_insert_with(|| {
                anomalies::artworks_en_place(&self.paths, &numero.classeur, self.source)
            });
            let en_place = numero
                .raretes
                .iter()
                .map(|r| {
                    (
                        r.clone(),
                        table.get(&(numero.set_code.clone(), r.clone())).cloned(),
                    )
                })
                .collect();
            let proposes = numero
                .artworks
                .iter()
                .map(|p| {
                    p.modele()
                        .and_then(|a| anomalies::fichier_image(a, self.source))
                })
                .collect();
            let mut ui = NumeroUi {
                coches: BTreeSet::new(),
                identiques: Vec::new(),
                en_place,
                proposes,
                numero,
            };
            ui.comparer(&self.paths.img_small());
            if let Some(coches) = anciennes.get(&cle(&ui.numero)) {
                ui.coches = coches
                    .iter()
                    .filter(|c| ui.proposition(c).is_some_and(|a| !a.corrige))
                    .cloned()
                    .collect();
            }
            self.numeros.push(ui);
        }
        if let Some(c) = courant {
            if let Some(i) = self.numeros.iter().position(|n| cle(&n.numero) == c) {
                self.courant = i;
            }
        }
        self.courant = self.courant.min(self.numeros.len().saturating_sub(1));
        self.cache.oublier_manquants();
    }

    /// Montre ce numéro, s'il fait partie des propositions.
    ///
    /// Rend `false` s'il n'y est pas — l'écran reste alors où il était.
    pub fn montrer(&mut self, set_code: &str) -> bool {
        match self
            .numeros
            .iter()
            .position(|n| n.numero.set_code.eq_ignore_ascii_case(set_code.trim()))
        {
            Some(i) => {
                self.aller_a(i);
                self.suivre_courant = true;
                true
            }
            None => false,
        }
    }

    /// Change de numéro.
    fn aller_a(&mut self, i: usize) {
        let i = i.min(self.numeros.len().saturating_sub(1));
        if i != self.courant {
            self.courant = i;
            self.rarete_choisie = None;
            self.rarete_survolee = None;
        }
    }

    /// Demande les aperçus manquants, par lots — le numéro regardé d'abord,
    /// puis les suivants.
    fn demander_les_apercus(&mut self) {
        let n = self.numeros.len();
        let mut lot = Vec::new();
        for decalage in 0..n {
            if lot.len() >= LOT_APERCUS {
                break;
            }
            let Some(numero) = self.numeros.get((self.courant + decalage) % n) else {
                continue;
            };
            for (p, fichier) in numero.numero.artworks.iter().zip(&numero.proposes) {
                let (Some(fichier), Some(modele)) = (fichier, p.modele()) else {
                    continue;
                };
                if !self.demandes.insert(fichier.clone()) {
                    continue;
                }
                if let Some(cible) =
                    anomalies::apercu_a_telecharger(&self.paths, modele, self.source)
                {
                    lot.push(cible);
                }
            }
        }
        self.a_telecharger.extend(lot);
    }

    fn dossier(&self) -> PathBuf {
        self.paths.img_small()
    }

    /// Une image de carte, en couleurs, à la hauteur demandée. Encadrée d'or
    /// si c'est une proposition.
    fn image(
        &mut self,
        ui: &mut egui::Ui,
        fichier: Option<&str>,
        hauteur: f32,
        proposee: bool,
    ) -> egui::Response {
        let dossier = self.dossier();
        // Plus d'atténuation : on compare des couleurs. `possedee = true`
        // est le rendu franc du cache.
        let texture = fichier.and_then(|f| {
            self.cache
                .texture(ui.ctx(), &dossier, Cle::nouvelle(f, true, false))
        });
        let taille = egui::vec2(hauteur * RATIO_CARTE, hauteur);
        let reponse = match texture {
            Some(t) => ui.add(
                egui::Image::new(&t)
                    .fit_to_exact_size(taille)
                    .corner_radius(4.0),
            ),
            None => {
                let (reponse, peintre) = ui.allocate_painter(taille, egui::Sense::hover());
                peintre.rect_filled(reponse.rect, 4.0, ui.visuals().extreme_bg_color);
                peintre.text(
                    reponse.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if fichier.is_some() { "⋯" } else { "—" },
                    egui::FontId::proportional(hauteur.clamp(12.0, 24.0)),
                    ui.visuals().weak_text_color(),
                );
                reponse
            }
        };
        if proposee {
            ui.painter().rect_stroke(
                reponse.rect.expand(3.0),
                6.0,
                egui::Stroke::new(if hauteur > 100.0 { 3.0 } else { 2.0 }, COULEUR_OR),
                egui::StrokeKind::Outside,
            );
        }
        reponse
    }

    /// `↑`/`↓`, `Espace`, `Entrée` — quand aucun champ n'a le focus.
    fn raccourcis(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || self.numeros.is_empty() {
            return;
        }
        let (haut, bas, espace, entree) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Space),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        if haut && self.courant > 0 {
            self.aller_a(self.courant - 1);
            self.suivre_courant = true;
        }
        if bas {
            self.aller_a(self.courant + 1);
            self.suivre_courant = true;
        }
        if espace {
            if let Some(n) = self.numeros.get_mut(self.courant) {
                n.basculer_tout();
            }
        }
        if entree {
            let lot = self
                .numeros
                .get(self.courant)
                .map(NumeroUi::cochees)
                .unwrap_or_default();
            if !lot.is_empty() {
                self.ajouter(&lot);
            }
        }
    }
}

impl eframe::App for EcranArtworks {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.raccourcis(racine.ctx());
        self.demander_les_apercus();
        self.barre_du_haut(racine);
        self.barre_du_bas(racine);
        if self.numeros.is_empty() {
            egui::CentralPanel::default().show(racine, |ui| self.rien_a_montrer(ui));
            return;
        }
        // Une seule carte : la liste n'aurait qu'une ligne.
        if !matches!(self.portee, Portee::Carte { .. }) {
            egui::Panel::left("liste-artworks")
                .resizable(true)
                .default_size(LARGEUR_LISTE)
                .size_range(300.0..=620.0)
                .show(racine, |ui| self.liste(ui));
        }
        egui::CentralPanel::default().show(racine, |ui| self.detail(ui));
    }
}

impl EcranArtworks {
    fn rien_a_montrer(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(96.0);
            ui.heading("Aucun autre artwork connu.");
            // Le message d'une carte affirmait « la base ne référence pas
            // d'autre illustration » — une affirmation sur les données, là
            // où la cause est le plus souvent un scan qui n'a pas tourné.
            ui.label(match &self.portee {
                Portee::Carte { .. } => {
                    "Aucune autre illustration connue pour ce tirage. Un scan peut en \
                     révéler d'autres, attestées par un set voisin."
                }
                _ => {
                    "Lancez un scan pour chercher les artworks alternatifs \
                     que vos classeurs pourraient avoir."
                }
            });
        });
    }

    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-artworks").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Retour").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                match &self.portee {
                    Portee::Carte { set_code, .. } => {
                        ui.heading("Artwork de la carte");
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(set_code).monospace());
                    }
                    Portee::Classeur(code) => {
                        ui.heading("Artworks alternatifs");
                        ui.add_space(12.0);
                        ui.label(code.as_str());
                    }
                    Portee::Tout => {
                        ui.heading("Artworks alternatifs");
                    }
                }
                ui.add_space(12.0);
                let cochees = self.cochees();
                ui.label(
                    egui::RichText::new(format!(
                        "{} numéro(s) · {} à ajouter · {cochees} cochée(s)",
                        self.numeros.len(),
                        self.restantes(),
                    ))
                    .weak(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            cochees > 0,
                            egui::Button::new(format!("Ajouter toute la sélection ({cochees})")),
                        )
                        .clicked()
                    {
                        let lot: Vec<Anomalie> =
                            self.numeros.iter().flat_map(NumeroUi::cochees).collect();
                        self.ajouter(&lot);
                    }
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(self.restantes() > 0, egui::Button::new("Tout cocher"))
                        .clicked()
                    {
                        for n in &mut self.numeros {
                            let bloc = n.a_cocher_en_bloc();
                            n.coches.extend(bloc);
                        }
                    }
                    ui.add_space(8.0);
                    // Le scan sert aussi sur une carte : ses propositions
                    // réunissent deux origines, dont la table qu'il remplit.
                    if ui
                        .button("🔍 Scanner")
                        .on_hover_text("Relit cardinfo.db en entier — quelques secondes")
                        .clicked()
                    {
                        self.scanner();
                    }
                });
            });
            if let Some((message, erreur)) = self.message.clone() {
                let couleur = if erreur {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.colored_label(couleur, message);
            }
            ui.add_space(8.0);
        });
    }

    fn barre_du_bas(&mut self, racine: &mut egui::Ui) {
        egui::Panel::bottom("aide-artworks").show(racine, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.small(
                    egui::RichText::new(
                        "Flèches haut / bas : numéro suivant · Espace : cocher tout le numéro · Entrée : ajouter \
                         ce qui est coché · survoler une rareté montre son image en place",
                    )
                    .weak(),
                );
            });
            ui.add_space(4.0);
        });
    }

    /// La liste de gauche : une ligne par numéro.
    fn liste(&mut self, ui: &mut egui::Ui) {
        let mut aller: Option<usize> = None;
        let mut basculer: Option<usize> = None;
        let suivre = std::mem::take(&mut self.suivre_courant);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(6.0);
                let mut classeur = String::new();
                for i in 0..self.numeros.len() {
                    let Some(n) = self.numeros.get(i).cloned() else {
                        continue;
                    };
                    if matches!(self.portee, Portee::Tout) && n.numero.classeur != classeur {
                        classeur.clone_from(&n.numero.classeur);
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(&classeur).strong().size(14.0));
                        ui.separator();
                    }
                    let actif = i == self.courant;
                    let visuels = ui.visuals().clone();
                    let cadre = egui::Frame::new()
                        .corner_radius(6.0)
                        .inner_margin(6.0)
                        .fill(if actif {
                            visuels.selection.bg_fill.linear_multiply(0.45)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(if actif {
                            visuels.selection.stroke
                        } else {
                            egui::Stroke::NONE
                        });
                    let mut rect_case = egui::Rect::NOTHING;
                    let reponse = cadre
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                rect_case =
                                    case_a_trois_etats(ui, n.etat(), n.numero.restantes() > 0);
                                ui.add_space(4.0);
                                let premier = n.proposes.first().cloned().flatten();
                                self.image(ui, premier.as_deref(), HAUTEUR_LISTE, false);
                                ui.add_space(6.0);
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&n.numero.nom).strong());
                                    let resume = match n.numero.artworks.as_slice() {
                                        [seul] => format!(
                                            "{} · Art {}",
                                            n.numero.set_code, seul.art_index
                                        ),
                                        plusieurs => format!(
                                            "{} · {} artworks proposés",
                                            n.numero.set_code,
                                            plusieurs.len()
                                        ),
                                    };
                                    ui.label(egui::RichText::new(resume).monospace().small().weak());
                                    let identiques =
                                        (0..n.proposes.len()).filter(|i| n.identique(*i)).count();
                                    if identiques > 0 {
                                        ui.small(
                                            egui::RichText::new(if identiques == n.proposes.len() {
                                                "≈ même image que celle en place".to_owned()
                                            } else {
                                                format!("≈ {identiques} image(s) identique(s) à l'en place")
                                            })
                                            .color(ui.visuals().warn_fg_color),
                                        );
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        for r in &n.numero.raretes {
                                            pastille(ui, r, n.rarete_cochee(r), n.rarete_posee(r));
                                        }
                                    });
                                    let restantes = n.numero.restantes();
                                    ui.small(
                                        egui::RichText::new(if restantes == 0 {
                                            "✓ tout est dans le classeur".to_owned()
                                        } else {
                                            format!("{} / {restantes} cochée(s)", n.cochees().len())
                                        })
                                        .weak(),
                                    );
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if reponse.clicked() {
                        let sur_la_case = reponse
                            .interact_pointer_pos()
                            .is_some_and(|p| rect_case.expand(4.0).contains(p));
                        if sur_la_case {
                            basculer = Some(i);
                        } else {
                            aller = Some(i);
                        }
                    }
                    if actif && suivre {
                        reponse.scroll_to_me(Some(egui::Align::Center));
                    }
                    ui.add_space(2.0);
                }
                ui.add_space(8.0);
            });
        if let Some(i) = basculer {
            if let Some(n) = self.numeros.get_mut(i) {
                n.basculer_tout();
            }
        }
        if let Some(i) = aller {
            self.aller_a(i);
        }
    }

    /// La comparaison : les images en grand, puis une ligne par rareté.
    fn detail(&mut self, ui: &mut egui::Ui) {
        let Some(n) = self.numeros.get(self.courant).cloned() else {
            return;
        };
        let total = self.numeros.len();

        // ── L'en-tête ───────────────────────────────────────────────────
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(&n.numero.nom);
            ui.add_space(10.0);
            ui.label(egui::RichText::new(&n.numero.set_code).monospace().weak());
            if total > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(self.courant + 1 < total, egui::Button::new("▶"))
                        .clicked()
                    {
                        self.aller_a(self.courant + 1);
                        self.suivre_courant = true;
                    }
                    ui.label(egui::RichText::new(format!("{} / {total}", self.courant + 1)).weak());
                    if ui
                        .add_enabled(self.courant > 0, egui::Button::new("◀"))
                        .clicked()
                    {
                        self.aller_a(self.courant - 1);
                        self.suivre_courant = true;
                    }
                });
            }
        });
        if n.numero.artworks.len() >= SEUIL_AVERTISSEMENT {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} artworks proposés : le scan propose toutes les illustrations connues de \
                     la carte, pas seulement celles imprimées dans ce set. Vérifiez sur la carte \
                     en main.",
                    n.numero.artworks.len()
                ),
            );
        }
        ui.add_space(8.0);

        // ── La rareté dont on montre l'image en place ────────────────────
        let premiere = n.numero.raretes.first().cloned().unwrap_or_default();
        let montree = self
            .rarete_survolee
            .clone()
            .or_else(|| self.rarete_choisie.clone())
            .filter(|r| n.numero.raretes.contains(r))
            .unwrap_or(premiere);
        let image_en_place = n.en_place.get(&montree).cloned().flatten();

        let hauteur = hauteur_des_images(
            ui.available_width(),
            ui.available_height(),
            n.numero.artworks.len(),
            n.numero.raretes.len(),
        );
        let hauteur_petite = HAUTEUR_RARETE;

        let mut survolee: Option<String> = None;
        let mut choisie: Option<String> = None;
        let mut bascules: Vec<Case> = Vec::new();
        let mut a_retirer: Option<Anomalie> = None;

        egui::ScrollArea::both()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(("grille-artworks", self.courant))
                    .spacing([18.0, 8.0])
                    .min_col_width(hauteur_petite * RATIO_CARTE)
                    .show(ui, |ui| {
                        // Ligne 0 : les images.
                        ui.allocate_space(egui::vec2(LARGEUR_LIBELLES, 0.0));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                etiquette(ui, "En place", false);
                                ui.label(egui::RichText::new(&montree).small());
                            });
                            ui.add_space(4.0);
                            self.image(ui, image_en_place.as_deref(), hauteur, false);
                        });
                        for (i, p) in n.numero.artworks.iter().enumerate() {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    if n.identique(i) {
                                        etiquette_alerte(ui, "Identique");
                                    } else {
                                        etiquette(ui, "Proposé", true);
                                    }
                                    ui.label(
                                        egui::RichText::new(format!("Art {}", p.art_index)).small(),
                                    );
                                });
                                ui.add_space(4.0);
                                let fichier = n.proposes.get(i).cloned().flatten();
                                self.image(ui, fichier.as_deref(), hauteur, !n.identique(i));
                                // Sous l'image, pas au-dessus : au-dessus, le
                                // texte décalait l'image et les deux cartes
                                // n'étaient plus à la même hauteur.
                                if n.identique(i) {
                                    ui.label(
                                        egui::RichText::new(
                                            "Même image que celle en place,\nsous un autre identifiant.\n\
                                             L'ajouter ferait un doublon.",
                                        )
                                        .small()
                                        .color(ui.visuals().warn_fg_color),
                                    )
                                    .on_hover_text(
                                        "YGOPRODeck garde deux identifiants pour une carte sortie en \
                                         OCG avant le TCG : le temporaire et le définitif. Même \
                                         illustration.",
                                    );
                                }
                            });
                        }
                        ui.end_row();

                        // Une ligne par rareté.
                        for r in &n.numero.raretes {
                            let en_place = n.en_place.get(r).cloned().flatten();
                            let libelle = ui
                                .vertical(|ui| {
                                    ui.set_width(LARGEUR_LIBELLES);
                                    let texte = egui::RichText::new(r).strong();
                                    let texte = if *r == montree {
                                        texte.color(ui.visuals().strong_text_color())
                                    } else {
                                        texte
                                    };
                                    ui.label(texte);
                                    ui.small(
                                        egui::RichText::new(format!(
                                            "en place : {}",
                                            origine_image(en_place.as_deref())
                                        ))
                                        .weak(),
                                    );
                                })
                                .response
                                .interact(egui::Sense::click());
                            let mini = self.image(ui, en_place.as_deref(), hauteur_petite, false);
                            if libelle.hovered() || mini.hovered() {
                                survolee = Some(r.clone());
                            }
                            if libelle.clicked() {
                                choisie = Some(r.clone());
                            }
                            for (i, p) in n.numero.artworks.iter().enumerate() {
                                match p.pour(r) {
                                    None => {
                                        ui.label(egui::RichText::new("—").weak())
                                            .on_hover_text("Pas proposé pour cette rareté");
                                    }
                                    Some(a) if a.corrige => {
                                        ui.horizontal(|ui| {
                                            ui.colored_label(
                                                ui.visuals().hyperlink_color,
                                                "✓ ajouté",
                                            );
                                            if ui
                                                .small_button("Retirer")
                                                .on_hover_text(
                                                    "Retire la ligne du classeur — sauf si vous \
                                                 l'avez marquée possédée",
                                                )
                                                .clicked()
                                            {
                                                a_retirer = Some(a.clone());
                                            }
                                        });
                                    }
                                    Some(_) => {
                                        let case = (i, r.clone());
                                        let mut coche = n.coches.contains(&case);
                                        if ui
                                            .checkbox(&mut coche, format!("Art {}", p.art_index))
                                            .changed()
                                        {
                                            bascules.push(case);
                                        }
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });

        // ── Les gestes du numéro ─────────────────────────────────────────
        ui.add_space(10.0);
        let lot = n.cochees();
        ui.horizontal(|ui| {
            let libelle = match lot.len() {
                0 => "Ajouter la sélection".to_owned(),
                1 => "Ajouter 1 artwork".to_owned(),
                k => format!("Ajouter {k} artworks"),
            };
            if ui
                .add_enabled(
                    !lot.is_empty(),
                    egui::Button::new(egui::RichText::new(libelle).color(COULEUR_OR)),
                )
                .clicked()
            {
                self.ajouter(&lot);
            }
            if n.numero.restantes() > 0 {
                let tout = n.etat() == Coche::Toutes;
                if ui
                    .button(if tout {
                        "Tout décocher"
                    } else {
                        "Tout cocher"
                    })
                    .clicked()
                {
                    if let Some(m) = self.numeros.get_mut(self.courant) {
                        m.basculer_tout();
                    }
                }
            }
        });
        ui.small(
            egui::RichText::new(
                "Chaque case cochée ajoute une ligne au classeur, non possédée, avec l'image de sa \
                 colonne — l'image YGOPRODeck, sans le rendu de la rareté.",
            )
            .weak(),
        );

        // ── Appliquer ce que le dessin a relevé ──────────────────────────
        self.rarete_survolee = survolee;
        if choisie.is_some() {
            self.rarete_choisie = choisie;
        }
        if let Some(m) = self.numeros.get_mut(self.courant) {
            for case in bascules {
                m.basculer(case);
            }
        }
        if let Some(a) = a_retirer {
            self.retirer(&a);
        }
    }

    /// Relance le scan et dit ce qu'il a trouvé.
    fn scanner(&mut self) {
        self.message = Some(match anomalies::scanner(&self.paths) {
            Ok(bilan) => (
                format!(
                    "{} artwork(s) alternatif(s) connu(s) — {} nouveau(x), {} retiré(s)",
                    bilan.total, bilan.ajoutees, bilan.retirees
                ),
                false,
            ),
            Err(e) => (e.to_string(), true),
        });
        self.demandes.clear();
        self.relire();
    }

    /// Ajoute un lot d'artworks au classeur.
    ///
    /// Le compte-rendu **distingue** ce qui a été posé de ce qui n'a pas pu
    /// l'être : sur les classeurs réels, deux propositions de `RA05` visent
    /// une rareté que le classeur ne porte pas du tout, et un « 171 posées »
    /// sec laisserait croire que les 173 sont réglées.
    fn ajouter(&mut self, lot: &[Anomalie]) {
        let (mut posees, mut deja, mut sans_temoin, mut erreurs) = (0, 0, 0, 0);
        for a in lot {
            match anomalies::corriger(&self.paths, a) {
                Ok(Correction::Ajoutee { .. }) => posees += 1,
                Ok(Correction::DejaPresente) => deja += 1,
                Ok(Correction::SansTemoin | Correction::ClasseurAbsent) => sans_temoin += 1,
                Err(_) => erreurs += 1,
            }
        }
        self.ecrit = posees > 0;
        let mut texte = format!("{posees} artwork(s) ajouté(s) au classeur");
        if deja > 0 {
            texte.push_str(&format!(" · {deja} déjà présent(s)"));
        }
        if sans_temoin > 0 {
            texte.push_str(&format!(
                " · {sans_temoin} sans ligne de ce tirage dans le classeur — rien d'où hériter"
            ));
        }
        if erreurs > 0 {
            texte.push_str(&format!(" · {erreurs} en erreur"));
        }
        self.message = Some((texte, erreurs > 0));
        self.relire();
    }

    /// Retire un artwork posé.
    fn retirer(&mut self, anomalie: &Anomalie) {
        self.message = Some(match anomalies::annuler(&self.paths, anomalie) {
            Ok(true) => {
                self.ecrit = true;
                (
                    format!(
                        "{} · {} retiré du classeur",
                        anomalie.missing_set_code, anomalie.missing_set_rarity
                    ),
                    false,
                )
            }
            Ok(false) => (
                format!(
                    "{} gardé — la ligne porte des exemplaires possédés",
                    anomalie.missing_set_code
                ),
                true,
            ),
            Err(e) => (e.to_string(), true),
        });
        self.relire();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Petits dessins
// ─────────────────────────────────────────────────────────────────────────────

/// Une case à trois états, peinte — rend son rectangle pour que la ligne
/// sache si c'est elle qu'on a cliquée.
fn case_a_trois_etats(ui: &mut egui::Ui, etat: Coche, active: bool) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let visuels = ui.visuals();
    let pleine = active && etat != Coche::Aucune;
    let fond = if pleine {
        visuels.selection.bg_fill
    } else {
        visuels.extreme_bg_color
    };
    let bord = if active {
        visuels.widgets.inactive.fg_stroke.color
    } else {
        visuels.weak_text_color()
    };
    ui.painter().rect_filled(rect, 3.0, fond);
    ui.painter().rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.5, bord),
        egui::StrokeKind::Inside,
    );
    let signe = match (active, etat) {
        (false, _) => "✓",
        (true, Coche::Toutes) => "✓",
        (true, Coche::Partielle) => "–",
        (true, Coche::Aucune) => "",
    };
    if !signe.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            signe,
            egui::FontId::proportional(13.0),
            if active {
                egui::Color32::WHITE
            } else {
                visuels.weak_text_color()
            },
        );
    }
    rect
}

/// Une rareté en pastille : dorée si cochée, barrée de vert si tout est posé.
fn pastille(ui: &mut egui::Ui, rarete: &str, cochee: bool, posee: bool) {
    let court = abreviation(rarete);
    let (texte, bord) = if posee {
        (
            egui::RichText::new(format!("✓ {court}"))
                .small()
                .color(ui.visuals().hyperlink_color),
            ui.visuals().hyperlink_color,
        )
    } else if cochee {
        (
            egui::RichText::new(court).small().color(COULEUR_OR),
            COULEUR_OR,
        )
    } else {
        (
            egui::RichText::new(court).small().weak(),
            ui.visuals().weak_text_color(),
        )
    };
    egui::Frame::new()
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .stroke(egui::Stroke::new(1.0, bord))
        .show(ui, |ui| {
            // Une pastille ne se coupe pas : « UR » sur deux lignes ne se lit
            // plus.
            ui.add(egui::Label::new(texte).extend());
        })
        .response
        .on_hover_text(rarete);
}

/// L'abréviation d'une rareté pour une pastille.
///
/// Celle d'YGOPRODeck quand elle existe ; sinon les initiales — « Quarter
/// Century Secret Rare » n'en a pas, et s'écrivait en entier dans la liste.
///
/// ```
/// use ygo_ui::artworks::abreviation;
/// assert_eq!(abreviation("Ultra Rare"), "UR");
/// assert_eq!(abreviation("Quarter Century Secret Rare"), "QCSR");
/// assert_eq!(abreviation("Rare"), "R");
/// ```
#[must_use]
pub fn abreviation(rarete: &str) -> String {
    if let Some(court) = ygo_core::rarity::canon::abreviation_ygoprodeck(rarete) {
        return court.to_owned();
    }
    let initiales: String = rarete
        .split_whitespace()
        .filter_map(|mot| mot.chars().next())
        .filter(|c| c.is_alphanumeric())
        .collect();
    if initiales.is_empty() {
        rarete.to_owned()
    } else {
        initiales.to_uppercase()
    }
}

/// « Identique » — une proposition qui n'en est pas une.
fn etiquette_alerte(ui: &mut egui::Ui, texte: &str) {
    let couleur = ui.visuals().warn_fg_color;
    egui::Frame::new()
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .stroke(egui::Stroke::new(1.0, couleur))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(texte).small().color(couleur));
        });
}

/// « En place » / « Proposé ».
fn etiquette(ui: &mut egui::Ui, texte: &str, proposee: bool) {
    let couleur = if proposee {
        COULEUR_OR
    } else {
        ui.visuals().weak_text_color()
    };
    egui::Frame::new()
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .stroke(egui::Stroke::new(1.0, couleur))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(texte).small().color(couleur));
        });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn anomalie(rarete: &str, art: &str, index: u32, corrige: bool) -> Anomalie {
        Anomalie {
            id: None,
            nom: "Maliss in Underground".to_owned(),
            art_a_image_uuid: "a".to_owned(),
            art_b_image_uuid: art.to_owned(),
            art_index: index,
            set_code_prefix: "LOCR-JP".to_owned(),
            missing_set_code: "LOCR-JP038".to_owned(),
            missing_set_rarity: rarete.to_owned(),
            image_url: String::new(),
            image_url_small: String::new(),
            image_id: Some(1),
            corrige,
        }
    }

    fn numero(anomalies: Vec<Anomalie>) -> NumeroUi {
        let numero = anomalies::regrouper(anomalies).remove(0);
        NumeroUi {
            en_place: HashMap::new(),
            proposes: vec![None; numero.artworks.len()],
            coches: BTreeSet::new(),
            identiques: vec![None; numero.artworks.len()],
            numero,
        }
    }

    /// Le cas de `LOCR-JP` : la proposition est la même image que celle en
    /// place. « Tout cocher » la saute ; la cocher reste possible, à la main.
    #[test]
    fn une_image_identique_n_est_pas_cochee_en_bloc() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("100438021.jpg"), b"maliss").unwrap();
        std::fs::write(tmp.path().join("68337209.jpg"), b"maliss").unwrap();
        std::fs::write(tmp.path().join("art3.jpg"), b"vraiment autre").unwrap();

        let mut n = numero(vec![
            anomalie(PSCR, "doublon", 2, false),
            anomalie(PSCR, "vrai", 3, false),
        ]);
        n.en_place
            .insert(PSCR.to_owned(), Some("100438021.jpg".to_owned()));
        n.proposes = vec![Some("68337209.jpg".to_owned()), Some("art3.jpg".to_owned())];
        n.comparer(tmp.path());

        assert!(n.identique(0), "même contenu, autre nom");
        assert!(!n.identique(1));
        n.basculer_tout();
        assert_eq!(n.cochees().len(), 1, "seul le vrai artwork");
        assert_eq!(n.cochees()[0].art_index, 3);
        n.basculer((0, PSCR.to_owned()));
        assert_eq!(n.cochees().len(), 2, "à la main, on peut toujours");
    }

    /// Une image pas encore téléchargée ne se juge pas : ni identique, ni
    /// différente — et elle reste cochable en bloc.
    #[test]
    fn une_image_absente_n_est_pas_declaree_identique() {
        let tmp = tempfile::tempdir().unwrap();
        let mut n = numero(vec![anomalie(PSCR, "b", 2, false)]);
        n.en_place
            .insert(PSCR.to_owned(), Some("la.jpg".to_owned()));
        n.proposes = vec![Some("pas_encore.jpg".to_owned())];
        n.comparer(tmp.path());
        assert_eq!(n.identiques[0], None);
        n.basculer_tout();
        assert_eq!(n.cochees().len(), 1);
    }

    const PSCR: &str = "Prismatic Secret Rare";
    const SCR: &str = "Secret Rare";
    const UR: &str = "Ultra Rare";

    /// La case à trois états : rien, une partie, tout — et le clic qui passe
    /// de l'un à l'autre comme dans l'explorateur.
    #[test]
    fn la_case_a_trois_etats_suit_la_selection() {
        let mut n = numero(vec![
            anomalie(PSCR, "b", 2, false),
            anomalie(SCR, "b", 2, false),
            anomalie(UR, "b", 2, false),
        ]);
        assert_eq!(n.etat(), Coche::Aucune);
        n.basculer((0, SCR.to_owned()));
        assert_eq!(n.etat(), Coche::Partielle);
        n.basculer_tout();
        assert_eq!(n.etat(), Coche::Toutes, "partiel → tout");
        assert_eq!(n.cochees().len(), 3);
        n.basculer_tout();
        assert_eq!(n.etat(), Coche::Aucune, "tout → rien");
    }

    /// Une proposition déjà posée ne se coche pas, et ne compte pas dans
    /// « tout ».
    #[test]
    fn ce_qui_est_pose_ne_se_coche_plus() {
        let mut n = numero(vec![
            anomalie(PSCR, "b", 2, true),
            anomalie(SCR, "b", 2, false),
        ]);
        n.basculer((0, PSCR.to_owned()));
        assert!(n.coches.is_empty(), "déjà dans le classeur");
        n.basculer_tout();
        assert_eq!(n.etat(), Coche::Toutes, "tout ce qui reste = la Secret");
        assert_eq!(n.cochees().len(), 1);
        assert!(n.rarete_posee(PSCR));
        assert!(!n.rarete_posee(SCR));
    }

    /// Deux artworks pour la même rareté : deux cases, cochables
    /// séparément — y compris toutes les deux.
    #[test]
    fn deux_artworks_sur_une_rarete_se_cochent_separement() {
        let mut n = numero(vec![
            anomalie(SCR, "art2", 2, false),
            anomalie(SCR, "art3", 3, false),
            anomalie(UR, "art3", 3, false),
        ]);
        assert_eq!(n.numero.artworks.len(), 2);
        n.basculer((1, SCR.to_owned()));
        let lot = n.cochees();
        assert_eq!(lot.len(), 1);
        assert_eq!(
            lot[0].art_index, 3,
            "la colonne de l'Art 3, pas celle de l'Art 2"
        );

        n.basculer((0, SCR.to_owned()));
        assert_eq!(n.cochees().len(), 2, "les deux sur la même rareté");
        assert!(n.rarete_cochee(SCR));
        assert!(!n.rarete_cochee(UR));
    }

    /// Une case qui n'existe pas — artwork non proposé pour cette rareté —
    /// ne se coche pas.
    #[test]
    fn une_case_absente_ne_se_coche_pas() {
        let mut n = numero(vec![
            anomalie(SCR, "art2", 2, false),
            anomalie(UR, "art3", 3, false),
        ]);
        n.basculer((0, UR.to_owned()));
        n.basculer((9, SCR.to_owned()));
        assert!(n.coches.is_empty());
    }

    /// Les images tiennent : jamais plus larges que leur colonne, jamais
    /// agrandies au-delà du fichier, jamais illisibles.
    #[test]
    fn la_taille_des_images_reste_dans_ses_bornes() {
        for (l, h, arts, rar) in [
            (1600.0, 1000.0, 1, 3),
            (1600.0, 1000.0, 8, 2),
            (900.0, 600.0, 4, 4),
            (200.0, 100.0, 1, 1),
        ] {
            let t = hauteur_des_images(l, h, arts, rar);
            assert!(
                (HAUTEUR_GRANDE_MIN..=HAUTEUR_GRANDE_MAX).contains(&t),
                "{t}"
            );
        }
        // Plus d'artworks, images plus petites.
        assert!(
            hauteur_des_images(1600.0, 1000.0, 8, 2) < hauteur_des_images(1600.0, 1000.0, 1, 2)
        );
        // Un seul artwork sur un écran courant : au moins trois fois la
        // vignette d'avant (105 points).
        assert!(hauteur_des_images(1100.0, 780.0, 1, 3) > 3.0 * 105.0);
    }

    #[test]
    fn l_origine_se_lit_sur_le_nom_du_fichier() {
        assert_eq!(origine_image(Some("12345.jpg")), "image YGOPRODeck");
        assert_eq!(origine_image(Some("Maiden-SDWD.png")), "scan Yugipedia");
        assert_eq!(origine_image(Some("")), "scan Yugipedia");
        assert_eq!(origine_image(None), "pas d'image");
    }
}
