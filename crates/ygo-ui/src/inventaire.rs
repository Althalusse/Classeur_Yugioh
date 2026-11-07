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
//! Les colonnes « Prix » (un exemplaire) et « Valeur » (× quantité), et la
//! valeur de ce qui est affiché dans la barre du haut, viennent de
//! [`ygo_app::cardmarket::valoriser`], chiffré sur un fil (2026-10-23).
//! Sans données Cardmarket, elles n'apparaissent pas.
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
//! | clic sur un prix | la page Cardmarket de la ligne, dans le navigateur |
//! | double-clic | la fiche de la carte ; `◀` `▶` et les flèches passent aux lignes voisines |
//!
//! Un clic droit sur une ligne **hors** de la sélection la sélectionne seule
//! d'abord : c'est ce que fait Windows, et c'est ce qui évite d'agir sur un
//! lot qu'on ne regarde plus.
//!
//! # La fiche, au double clic — 2026-11-05
//!
//! Partout ailleurs dans l'application, ouvrir une carte, c'est ouvrir sa
//! fiche : illustration, textes, prix, bouton Cardmarket. L'inventaire fait
//! de même, au **double** clic — le clic simple y sélectionne, c'est la
//! raison d'être de l'écran. C'est la fiche du classeur
//! ([`crate::fiche::EtatFiche`], lue par [`ygo_app::fiche::lire`]), à ceci
//! près que ses voisines sont les lignes **de la liste affichée**, dans
//! l'ordre du tri et des filtres : on feuillette ce qu'on a sous les yeux. La
//! ligne montrée devient la sélection, et le tableau défile jusqu'à elle :
//! la fiche fermée, on est sur la dernière carte regardée.
//!
//! Le double clic se fait sans `Ctrl` ni `Maj`, qui gardent leur sens de
//! sélection. Sur le prix, il n'ouvre rien de plus : le premier clic a déjà
//! ouvert la page Cardmarket. Fiche ouverte, `Échap` la ferme et ne
//! désélectionne rien ; `Ctrl+A` et `Suppr` attendent qu'elle soit fermée.
//!
//! # La page Cardmarket d'une ligne — 2026-11-05
//!
//! Le prix d'une ligne — ou son tiret, quand le prix n'est pas sûr — ouvre
//! sa page Cardmarket ; le clic droit la propose aussi, nommée par le numéro
//! de la ligne cliquée, puisque le reste du menu sert toute la sélection.
//! C'est la page du bouton de la fiche : la même offre, celle dont le prix
//! remplit la colonne ([`ygo_app::cardmarket::Valorisation::url`]), sur les
//! annonces dans la langue du classeur. L'application n'envoie rien à
//! `www.cardmarket.com` : elle passe un lien au navigateur.
//!
//! Le montant ne capte pas le clic : la ligne le reçoit, comme partout dans
//! le tableau, et regarde s'il est tombé sur le montant. Le glisser, le clic
//! droit et `Ctrl`/`Maj` gardent donc leur sens ; seul un clic simple sur le
//! montant ouvre la page — sans toucher à la sélection.
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
use ygo_app::cardmarket::{Cible, Offre, Valorisation};
use ygo_app::exemplaires::{Demande, Etat, Exemplaire};
use ygo_app::inventaire::{self, Carte, Colonne, Filtre};
use ygo_app::scanflip;
use ygo_core::cardmarket::LangueDesAnnonces;
use ygo_core::config::{Config, SourceImage};
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
    /// Les cartes dépliées en leurs exemplaires, par adresse.
    deplies: BTreeSet<(String, i64)>,

    /// L'import/export, partagé avec l'accueil.
    csv: PanneauCsv,
    /// Les classeurs dont l'utilisateur a demandé la création.
    ///
    /// L'écran ne les crée pas : la création va sur le réseau et vit sur
    /// le fil de travail de l'application, comme celle du sélecteur.
    creations: Vec<String>,

    /// Les prix Cardmarket des lignes possédées, une fois chiffrés —
    /// `None` avant, ou sans données Cardmarket : les colonnes « Prix » et
    /// « Valeur » n'apparaissent qu'avec eux.
    valorisation: Option<ygo_app::cardmarket::Valorisation>,
    /// Le chiffrage en cours, sur son fil.
    chiffrage: crate::valeur::Calcul<Option<ygo_app::cardmarket::Valorisation>>,

    /// La fiche d'une ligne, ouverte au double clic (2026-11-05).
    fiche: crate::fiche::EtatFiche,
    /// L'adresse de la ligne qu'elle montre : la fiche ne connaît que son
    /// `rowid`, et deux classeurs ont chacun leur ligne 1.
    ligne_de_la_fiche: Option<(String, i64)>,
    /// Les illustrations de la fiche, décodées une fois.
    images: crate::images::Cache,
    /// La source des illustrations — celle des Options, comme au classeur.
    source: SourceImage,
    /// La ligne que le tableau doit amener sous les yeux à la prochaine
    /// image : celle que la fiche vient de montrer.
    a_montrer: Option<(String, i64)>,

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
        let source = Config::charger(paths.app_config()).source_image();
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
            deplies: BTreeSet::new(),
            csv,
            creations: Vec::new(),
            valorisation: None,
            chiffrage: crate::valeur::Calcul::default(),
            fiche: crate::fiche::EtatFiche::default(),
            ligne_de_la_fiche: None,
            images: crate::images::Cache::default(),
            source,
            a_montrer: None,
            message: None,
            retour: false,
            duree_lecture: std::time::Duration::ZERO,
        };
        ecran.relire();
        ecran.chiffrer();
        ecran
    }

    /// Relit la base et recalcule ce qui en dépend.
    fn relire(&mut self) {
        let debut = std::time::Instant::now();
        self.cartes = inventaire::lister(&self.paths);
        self.duree_lecture = debut.elapsed();
        self.classeurs = inventaire::valeurs(&self.cartes, |c| &c.classeur);
        self.raretes = inventaire::valeurs(&self.cartes, |c| &c.rarete);
        // Les états des exemplaires réglés à part comptent aussi : le filtre
        // « PL » doit pouvoir trouver la carte dont deux exemplaires le sont.
        self.etats = inventaire::qualites(&self.cartes);
        self.poser_les_prix();
        self.trier();
        // Une ligne possédée depuis le chiffrage — un import — n'a pas de
        // prix : on rechiffre, sur le fil.
        let a_rechiffrer = self.valorisation.as_ref().is_some_and(|v| {
            self.cartes
                .iter()
                .any(|c| !v.chiffree(&c.classeur, c.rowid))
        });
        if a_rechiffrer && !self.chiffrage.en_cours() {
            self.chiffrer();
        }
        // Une ligne retirée ne doit pas rester sélectionnée : l'action
        // suivante porterait sur une adresse qui n'est plus dans la liste.
        let vivantes: BTreeSet<(String, i64)> = self
            .cartes
            .iter()
            .map(|c| (c.classeur.clone(), c.rowid))
            .collect();
        self.selection.retain(|a| vivantes.contains(a));
        self.deplies.retain(|a| vivantes.contains(a));
        self.ancre = None;
        self.glisse = None;
        // La fiche ouverte se relit — sa ligne « Possédée » suit la quantité
        // — ou se ferme, si sa ligne a quitté l'inventaire.
        if let Some(adresse) = self.ligne_de_la_fiche.clone() {
            if vivantes.contains(&adresse) {
                self.ouvrir_la_fiche(&adresse);
            } else {
                self.fermer_la_fiche();
            }
        }
    }

    /// Ouvre la fiche d'une ligne.
    ///
    /// Une lecture ratée n'ouvre rien, comme dans le classeur : mieux vaut ne
    /// rien voir se passer qu'une fenêtre vide. La trace, elle, est écrite.
    fn ouvrir_la_fiche(&mut self, adresse: &(String, i64)) {
        let (classeur, rowid) = (adresse.0.as_str(), adresse.1);
        match ygo_app::fiche::lire(&self.paths, classeur, rowid, self.source) {
            Ok(Some(f)) => {
                self.fiche.ouvrir(f);
                self.ligne_de_la_fiche = Some(adresse.clone());
            }
            Ok(None) => tracing::warn!(classeur, rowid, "fiche : ligne introuvable"),
            Err(e) => tracing::warn!(classeur, rowid, erreur = %e, "fiche : lecture impossible"),
        }
    }

    /// Ferme la fiche.
    fn fermer_la_fiche(&mut self) {
        self.fiche.fermer();
        self.ligne_de_la_fiche = None;
    }

    /// Dessine la fiche par-dessus le tableau, et obéit à ce qu'elle demande.
    fn montrer_la_fiche(&mut self, ctx: &egui::Context) {
        if !self.fiche.est_ouverte() {
            return;
        }
        let dossier = self.paths.img_small();
        match self
            .fiche
            .afficher(ctx, &mut self.images, &dossier, self.francais)
        {
            crate::fiche::Demande::Rien => {}
            crate::fiche::Demande::Fermer => self.fermer_la_fiche(),
            crate::fiche::Demande::Precedente => self.deplacer_la_fiche(-1),
            crate::fiche::Demande::Suivante => self.deplacer_la_fiche(1),
        }
    }

    /// Passe à la ligne voisine **de la liste affichée** — son tri, ses
    /// filtres —, sans boucler aux extrémités ([`crate::fiche::voisine`]).
    /// La ligne montrée devient la sélection, et le tableau défile jusqu'à
    /// elle.
    fn deplacer_la_fiche(&mut self, pas: isize) {
        let Some(actuelle) = self.ligne_de_la_fiche.clone() else {
            return;
        };
        let adresses: Vec<(String, i64)> = self
            .visibles()
            .iter()
            .map(|c| (c.classeur.clone(), c.rowid))
            .collect();
        // Une ligne que les filtres cachent depuis : on ne sait plus où l'on
        // est, la fiche reste sur place.
        let Some(i) = adresses.iter().position(|a| *a == actuelle) else {
            return;
        };
        let vise = crate::fiche::voisine(i, adresses.len(), pas);
        if vise == i {
            return;
        }
        let Some(adresse) = adresses.get(vise).cloned() else {
            return;
        };
        self.ouvrir_la_fiche(&adresse);
        self.selection = BTreeSet::from([adresse.clone()]);
        self.ancre = Some(vise);
        self.a_montrer = Some(adresse);
    }

    /// Des illustrations sont arrivées : celles qui manquaient ont de
    /// nouveau leur chance (cf. [`crate::images::Cache::oublier_manquants`]).
    pub fn rafraichir_images(&mut self) {
        self.images.oublier_manquants();
    }

    fn trier(&mut self) {
        inventaire::trier(
            &mut self.cartes,
            self.colonne,
            self.descendant,
            self.francais,
        );
    }

    /// Chiffre les lignes possédées sur un fil : l'écran s'affiche sans
    /// attendre les prix — un dixième de seconde d'ordinaire.
    fn chiffrer(&mut self) {
        let paths = self.paths.clone();
        let codes = self.classeurs.clone();
        self.chiffrage = crate::valeur::Calcul::lancer("ygo-prix-inventaire", move || {
            ygo_app::cardmarket::valoriser(&paths, &codes).unwrap_or_else(|e| {
                tracing::warn!(erreur = %e, "inventaire : prix non calculés");
                None
            })
        });
    }

    /// Relève les prix s'ils sont arrivés, et les pose sur les cartes.
    fn relever_les_prix(&mut self, ctx: &egui::Context) {
        if let Some(valorisation) = self.chiffrage.relever() {
            self.valorisation = valorisation;
            // Sans prix, plus de colonne de prix : on ne trie plus dessus.
            if self.valorisation.is_none() && self.colonne.de_prix() {
                self.colonne = Colonne::default();
                self.descendant = false;
            }
            self.poser_les_prix();
            self.trier();
        }
        if self.chiffrage.en_cours() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    /// Pose les prix connus sur les cartes.
    fn poser_les_prix(&mut self) {
        if let Some(v) = &self.valorisation {
            inventaire::poser_les_prix(&mut self.cartes, |classeur, rowid| v.prix(classeur, rowid));
        }
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

    /// Exécute ce qu'une ligne d'exemplaire a demandé.
    fn agir_sur_exemplaire(&mut self, adresse: &(String, i64), demande: &Demande) {
        match inventaire::agir_sur_exemplaires(&self.paths, adresse, demande) {
            Ok(_) => {
                self.message = None;
                self.relire();
            }
            Err(e) => self.message = Some((e.to_string(), true)),
        }
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
    ///
    /// Ni pendant que la fiche est ouverte : `Échap` est à elle — il la
    /// ferme, il ne doit pas aussi vider la sélection —, et un retrait
    /// demandé derrière une fenêtre qui cache le tableau se ferait à
    /// l'aveugle.
    fn raccourcis(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || self.retrait || self.fiche.est_ouverte() {
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

/// Une rangée du tableau : une carte, ou l'un de ses exemplaires quand elle
/// est dépliée.
#[derive(Debug, Clone)]
enum Rangee {
    /// La carte, par son indice dans les cartes visibles.
    Carte(usize),
    /// Un exemplaire de la carte d'indice `carte`, numéroté à partir de 1.
    Exemplaire {
        carte: usize,
        numero: usize,
        exemplaire: Exemplaire,
        etat: Etat,
    },
}

/// La largeur de chaque colonne du tableau.
fn largeur_de(colonne: Colonne) -> Column {
    match colonne {
        Colonne::Nom => Column::auto().at_least(220.0).resizable(true),
        Colonne::Code => Column::auto().at_least(120.0),
        Colonne::Rarete => Column::auto().at_least(150.0),
        Colonne::Quantite | Colonne::Surplus => Column::auto().at_least(60.0),
        Colonne::Prix | Colonne::Qualite => Column::auto().at_least(70.0),
        Colonne::Valeur | Colonne::Classeur => Column::auto().at_least(90.0),
    }
}

/// L'écart entre un montant et la colonne qui le suit.
const MARGE_EUROS: f32 = 12.0;

/// Ce que dit le tiret d'une ligne sans prix sûr.
const SANS_PRIX_SUR: &str =
    "Pas de prix sûr : version incertaine, ou carte non retrouvée chez Cardmarket";

/// Ce que propose le clic droit, et ce que le survol d'un prix annonce : la
/// page qui s'ouvrira, nommée par le numéro de la ligne — le reste du menu
/// sert toute la sélection, le lien ne vaut que pour la ligne cliquée. Les
/// mêmes cas que le bouton de la fiche ([`crate::fiche::texte_du_bouton`]).
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// use ygo_ui::inventaire::texte_du_lien;
/// let mut o = Offre { cible: Cible::Produit(1), version: Some("V.3 · Ultimate Rare".into()),
///     prix: PrixOffre::Aucun, raison: Raison::Ordonnee, extension: None,
///     prix_du: String::new() };
/// assert_eq!(
///     texte_du_lien(&o, "RA02-EN001"),
///     "Voir RA02-EN001 sur Cardmarket · V.3 Ultimate Rare"
/// );
/// o.version = None;
/// assert_eq!(texte_du_lien(&o, "RA02-EN001"), "Voir RA02-EN001 sur Cardmarket");
/// o.cible = Cible::Recherche { nom: "Rescue Cat".into(), id_extension: Some(5604) };
/// assert_eq!(
///     texte_du_lien(&o, "RA02-EN001"),
///     "Voir les versions de RA02-EN001 sur Cardmarket"
/// );
/// o.cible = Cible::Recherche { nom: "Rescue Cat".into(), id_extension: None };
/// assert_eq!(texte_du_lien(&o, "RA02-EN001"), "Chercher « Rescue Cat » sur Cardmarket");
/// o.cible = Cible::Numero("CORI-JP001".into());
/// assert_eq!(texte_du_lien(&o, "CORI-JP001"), "Chercher CORI-JP001 sur Cardmarket");
/// ```
#[must_use]
pub fn texte_du_lien(offre: &Offre, set_code: &str) -> String {
    match &offre.cible {
        Cible::Produit(_) => match &offre.version {
            Some(version) => format!(
                "Voir {set_code} sur Cardmarket · {}",
                version.replacen(" · ", " ", 1)
            ),
            None => format!("Voir {set_code} sur Cardmarket"),
        },
        Cible::Recherche {
            id_extension: Some(_),
            ..
        } => format!("Voir les versions de {set_code} sur Cardmarket"),
        Cible::Recherche {
            nom,
            id_extension: None,
        } => format!("Chercher « {nom} » sur Cardmarket"),
        Cible::Numero(numero) => format!("Chercher {numero} sur Cardmarket"),
    }
}

/// La page Cardmarket d'une ligne chiffrée : son offre — celle dont le prix
/// remplit la colonne, celle de sa fiche — et l'adresse à ouvrir.
#[derive(Debug, Clone)]
struct Lien<'a> {
    offre: &'a Offre,
    set_code: &'a str,
    annonces: LangueDesAnnonces,
    url: String,
}

impl<'a> Lien<'a> {
    /// Le lien d'une carte — `None` sans prix chiffrés, ou pour une ligne
    /// possédée depuis le chiffrage : elle l'aura au suivant.
    fn de(
        valorisation: Option<&'a Valorisation>,
        carte: &'a Carte,
        francais: bool,
    ) -> Option<Self> {
        let v = valorisation?;
        Some(Self {
            offre: v.offre(&carte.classeur, carte.rowid)?,
            set_code: &carte.set_code,
            annonces: v.annonces(&carte.classeur),
            url: v.url(&carte.classeur, carte.rowid, &carte.set_code, francais)?,
        })
    }

    /// Le texte du menu ([`texte_du_lien`]).
    fn texte(&self) -> String {
        texte_du_lien(self.offre, self.set_code)
    }

    /// L'info-bulle : la version à ouvrir quand le lien mène à une
    /// recherche, puis ce que dit celle du bouton de la fiche.
    ///
    /// De l'indication que la fiche écrit sous son bouton, seule la version
    /// à ouvrir est reprise : le reste — pourquoi une recherche — est la
    /// raison que l'aide du bouton donne déjà, et la bulle la disait deux
    /// fois (CORI-JP067, Xvfb, 2026-11-05).
    fn aide(&self) -> String {
        let mut lignes = Vec::new();
        if let (false, Some(version)) = (self.offre.est_directe(), &self.offre.version) {
            lignes.push(format!("Version à ouvrir : {version}"));
        }
        lignes.push(crate::fiche::aide_du_lien(self.offre));
        if let Some(filtre) = crate::fiche::filtre_du_lien(self.offre, self.set_code, self.annonces)
        {
            lignes.push(filtre);
        }
        lignes.join("\n")
    }

    /// Ouvre la page dans le navigateur, dans un nouvel onglet.
    fn ouvrir(&self, ctx: &egui::Context) {
        ctx.open_url(egui::OpenUrl::new_tab(self.url.clone()));
    }
}

/// Un montant, aligné à droite pour que les colonnes se lisent — un tiret
/// quand il n'y a pas de prix sûr.
///
/// Avec un lien, le montant — ou le tiret — se souligne au survol, prend la
/// main des liens et dit ce qu'un clic ouvrira. Il ne capte pas le clic pour
/// autant : la cellule rend le rectangle du texte, et la ligne, qui reçoit
/// le clic, regarde s'il y est tombé (cf. [`EcranInventaire::table`]).
fn cellule_euros(
    ui: &mut egui::Ui,
    centimes: Option<u64>,
    lien: Option<&Lien<'_>>,
) -> Option<egui::Rect> {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(MARGE_EUROS);
        let Some(lien) = lien else {
            match centimes {
                Some(c) => {
                    ui.label(ygo_app::cardmarket::euros(c));
                }
                None => {
                    ui.weak("—").on_hover_text(SANS_PRIX_SUR);
                }
            }
            return None;
        };
        let (texte, couleur) = match centimes {
            Some(c) => (ygo_app::cardmarket::euros(c), ui.visuals().text_color()),
            None => ("—".to_owned(), ui.visuals().weak_text_color()),
        };
        // Dessiné comme egui dessine ses liens, mais dans la couleur du
        // tableau : une colonne de prix bleus crierait plus qu'elle ne dirait.
        let (position, galley, reponse) = egui::Label::new(texte).layout_in_ui(ui);
        if ui.is_rect_visible(reponse.rect) {
            let soulignement = if reponse.hovered() {
                egui::Stroke::new(1.0, couleur)
            } else {
                egui::Stroke::NONE
            };
            ui.painter().add(
                egui::epaint::TextShape::new(position, galley, couleur)
                    .with_underline(soulignement),
            );
        }
        let rect = reponse.rect;
        reponse
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_ui(|ui| {
                if centimes.is_none() {
                    ui.label(SANS_PRIX_SUR);
                    ui.add_space(4.0);
                }
                ui.label(egui::RichText::new(format!("Clic : {}", lien.texte())).strong());
                ui.label(lien.aide());
            });
        Some(rect)
    })
    .inner
}

/// Les rangées du tableau : chaque carte, et sous une carte dépliée de
/// plusieurs exemplaires, chacun d'eux.
fn rangees(visibles: &[Carte], deplies: &BTreeSet<(String, i64)>) -> Vec<Rangee> {
    let mut affichage = Vec::with_capacity(visibles.len());
    for (i, c) in visibles.iter().enumerate() {
        affichage.push(Rangee::Carte(i));
        if c.quantite > 1 && deplies.contains(&(c.classeur.clone(), c.rowid)) {
            for (k, (exemplaire, etat)) in c.exemplaires().liste().into_iter().enumerate() {
                affichage.push(Rangee::Exemplaire {
                    carte: i,
                    numero: k + 1,
                    exemplaire,
                    etat,
                });
            }
        }
    }
    affichage
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

/// Le menu d'un clic droit : il agit sur **toute** la sélection — sauf la
/// page Cardmarket, qui est celle de la ligne cliquée et le dit.
fn menu_selection(ui: &mut egui::Ui, n: usize, lien: Option<&Lien<'_>>, geste: &mut Option<Geste>) {
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
    if let Some(lien) = lien {
        if ui.button(lien.texte()).on_hover_text(lien.aide()).clicked() {
            lien.ouvrir(ui.ctx());
            ui.close();
        }
    }
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
        self.relever_les_prix(racine.ctx());
        self.barre_du_haut(racine);
        self.barre_des_filtres(racine);
        self.barre_du_bas(racine);
        let retour = self.csv.afficher(racine, &paths, self.francais);
        self.appliquer_retour(retour);
        self.table(racine);
        self.confirmation_retrait(racine.ctx());
        // En dernier : la fiche se pose par-dessus tout le reste.
        self.montrer_la_fiche(racine.ctx());
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
                let mut resume = format!(
                    "{} ligne(s) · {} exemplaire(s) · {} playset(s) · {} en surplus",
                    t.lignes, t.exemplaires, t.playsets, t.surplus
                );
                // La valeur de ce qui est affiché : filtrer sur un classeur
                // ou une rareté la chiffre d'un coup d'œil.
                let valeur = self.valorisation.as_ref().and_then(|v| {
                    let valeur = inventaire::valeur(&visibles);
                    let montant = crate::valeur::montant(&valeur)?;
                    Some((montant, crate::valeur::detail(&valeur, &v.prix_du)))
                });
                match valeur {
                    Some((montant, detail)) => {
                        resume.push_str(&format!(" · {montant}"));
                        ui.label(resume).on_hover_text(detail);
                    }
                    None => {
                        ui.label(resume);
                    }
                }

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
                // Le prix ne mène à Cardmarket qu'une fois chiffré.
                let prix = if self.valorisation.is_some() {
                    " ; clic sur un prix pour sa page Cardmarket"
                } else {
                    ""
                };
                ui.horizontal(|ui| {
                    ui.small(format!(
                        "{} carte(s) lues en {:?} — clic, Ctrl+clic, Maj+clic ou glisser \
                         pour sélectionner ; Ctrl+A pour tout ; clic droit pour agir ; \
                         double-clic pour la fiche{prix}",
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
            let mut deplier: Option<(String, i64)> = None;
            let mut demande_exemplaire: Option<((String, i64), Demande)> = None;
            // La page Cardmarket qu'un clic sur un prix a demandée.
            let mut a_ouvrir: Option<String> = None;
            // La ligne dont un double clic a demandé la fiche.
            let mut fiche_demandee: Option<(String, i64)> = None;
            let valorisation = self.valorisation.as_ref();
            let francais = self.francais;

            // Les rangées affichées : chaque carte, et sous une carte
            // dépliée, chacun de ses exemplaires.
            let affichage = rangees(&visibles, &self.deplies);
            let hauteurs: Vec<f32> = affichage
                .iter()
                .map(|r| match r {
                    Rangee::Carte(_) => 22.0,
                    Rangee::Exemplaire { .. } => 26.0,
                })
                .collect();
            let hauteurs = hauteurs.into_iter();
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

            // « Prix » et « Valeur » ne viennent qu'avec les prix : sans
            // données Cardmarket, deux colonnes de tirets ne diraient rien.
            let avec_prix = self.valorisation.is_some();
            let colonnes: Vec<Colonne> = Colonne::toutes()
                .into_iter()
                .filter(|c| avec_prix || !c.de_prix())
                .collect();
            // Les cellules prennent la disposition du parent si on ne leur
            // en donne pas : de haut en bas, dans un panneau. Le triangle de
            // la première colonne passait alors AU-DESSUS du nom, chaque
            // ligne montait à 35 points au lieu de 22, et le nom se lisait à
            // mi-chemin entre son code et celui de la ligne suivante
            // (capture du 2026-10-23). De gauche à droite, centrées.
            let mut table = TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .sense(egui::Sense::click_and_drag());
            // La ligne que la fiche vient de montrer, amenée sous les yeux —
            // une fois : ensuite, la molette est à l'utilisateur.
            if let Some(adresse) = self.a_montrer.take() {
                let rang = affichage.iter().position(|r| {
                    matches!(r, Rangee::Carte(i)
                        if visibles.get(*i).is_some_and(|c| c.classeur == adresse.0 && c.rowid == adresse.1))
                });
                if let Some(rang) = rang {
                    table = table.scroll_to_row(rang, None);
                }
            }
            for colonne in &colonnes {
                table = table.column(largeur_de(*colonne));
            }
            table
                .column(Column::remainder())
                .header(24.0, |mut entete| {
                    for colonne in colonnes.iter().copied() {
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
                            let titre =
                                egui::RichText::new(format!("{}{marque}", colonne.libelle()))
                                    .strong();
                            // Les montants sont alignés à droite : leur
                            // en-tête aussi, au-dessus des unités.
                            let clique = if colonne.de_prix() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add_space(MARGE_EUROS);
                                        ui.button(titre).clicked()
                                    },
                                )
                                .inner
                            } else {
                                ui.button(titre).clicked()
                            };
                            if clique {
                                a_trier = Some(colonne);
                            }
                        });
                    }
                    entete.col(|ui| {
                        ui.label(egui::RichText::new("Variante").strong());
                    });
                })
                .body(|corps| {
                    corps.heterogeneous_rows(hauteurs, |mut ligne| {
                        let Some(rangee) = affichage.get(ligne.index()) else {
                            return;
                        };
                        let (i, sous) = match rangee {
                            Rangee::Carte(i) => (*i, None),
                            Rangee::Exemplaire {
                                carte,
                                numero,
                                exemplaire,
                                etat,
                            } => (*carte, Some((*numero, *exemplaire, etat))),
                        };
                        let Some(c) = visibles.get(i) else {
                            return;
                        };
                        let adresse = (c.classeur.clone(), c.rowid);

                        // Une rangée d'exemplaire : ses réglages, et rien
                        // d'autre — elle ne se sélectionne pas.
                        if let Some((numero, exemplaire, etat)) = sous {
                            let a_part = matches!(exemplaire, Exemplaire::APart(_));
                            let cle = (c.classeur.as_str(), c.rowid, numero);
                            ligne.col(|ui| {
                                ui.add_space(22.0);
                                let texte = format!("Exemplaire {numero}");
                                let r = if a_part {
                                    ui.label(egui::RichText::new(texte).strong())
                                } else {
                                    ui.weak(texte)
                                };
                                r.on_hover_text(if a_part {
                                    "Réglé à part"
                                } else {
                                    "Standard : il suit l'état de la carte"
                                });
                            });
                            ligne.col(|ui| {
                                let mut qualite = etat.qualite.clone();
                                if crate::exemplaires::choix_etat(ui, (cle, "etat"), &mut qualite) {
                                    demande_exemplaire = Some((
                                        adresse.clone(),
                                        Demande::Modifier(
                                            exemplaire,
                                            Etat::nouveau(&qualite, &etat.edition),
                                        ),
                                    ));
                                }
                            });
                            ligne.col(|ui| {
                                let mut edition = etat.edition.clone();
                                if crate::exemplaires::choix_edition(
                                    ui,
                                    (cle, "edition"),
                                    &mut edition,
                                ) {
                                    demande_exemplaire = Some((
                                        adresse.clone(),
                                        Demande::Modifier(
                                            exemplaire,
                                            Etat::nouveau(&etat.qualite, &edition),
                                        ),
                                    ));
                                }
                            });
                            ligne.col(|_| {});
                            ligne.col(|_| {});
                            if avec_prix {
                                ligne.col(|_| {});
                                ligne.col(|_| {});
                            }
                            ligne.col(|ui| {
                                if ui
                                    .small_button("Retirer")
                                    .on_hover_text(
                                        "Retirer cet exemplaire : la quantité baisse d'un",
                                    )
                                    .clicked()
                                {
                                    demande_exemplaire =
                                        Some((adresse.clone(), Demande::Retirer(exemplaire)));
                                }
                            });
                            ligne.col(|_| {});
                            ligne.col(|_| {});
                            return;
                        }

                        ligne.set_selected(self.selection.contains(&adresse));
                        let deplie = self.deplies.contains(&adresse);
                        let lien = Lien::de(valorisation, c, francais);
                        // Le texte du prix, quand il mène à Cardmarket.
                        let mut texte_du_prix: Option<egui::Rect> = None;

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
                            // Le triangle qui déplie : seulement là où il y a
                            // plusieurs exemplaires à montrer. L'espace est
                            // gardé ailleurs, pour que les noms s'alignent.
                            let (_, triangle) = ui.allocate_exact_size(
                                egui::vec2(14.0, 14.0),
                                if c.quantite > 1 {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::hover()
                                },
                            );
                            if c.quantite > 1 {
                                egui::collapsing_header::paint_default_icon(
                                    ui,
                                    if deplie { 1.0 } else { 0.0 },
                                    &triangle,
                                );
                                if triangle
                                    .on_hover_text(if deplie {
                                        "Replier"
                                    } else {
                                        "Voir chaque exemplaire"
                                    })
                                    .clicked()
                                {
                                    deplier = Some(adresse.clone());
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
                        if avec_prix {
                            ligne.col(|ui| {
                                texte_du_prix =
                                    cellule_euros(ui, c.prix.map(u64::from), lien.as_ref());
                            });
                            ligne.col(|ui| {
                                cellule_euros(ui, c.valeur(), None);
                            });
                        }
                        ligne.col(|ui| {
                            let exemplaires = c.exemplaires();
                            if !exemplaires.homogene() {
                                // Des exemplaires d'états différents : le
                                // décompte, et le détail au survol.
                                let detail: Vec<String> = exemplaires
                                    .groupes()
                                    .iter()
                                    .map(|(e, n)| {
                                        format!(
                                            "{n} × {} — {}",
                                            if e.qualite.is_empty() {
                                                "—".to_owned()
                                            } else {
                                                libelle_qualite(&e.qualite)
                                            },
                                            crate::exemplaires::libelle_edition(&e.edition)
                                        )
                                    })
                                    .collect();
                                ui.label(crate::exemplaires::resume(&exemplaires))
                                    .on_hover_text(detail.join("\n"));
                            } else if c.qualite.is_empty() {
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
                            // Un clic simple sur le prix ouvre la page, sans
                            // toucher à la sélection ; avec `Ctrl` ou `Maj`,
                            // il sélectionne, comme ailleurs sur la ligne. Les
                            // clics suivants d'un double ou d'un triple clic
                            // n'ouvrent pas d'autre onglet.
                            let sur_le_prix = texte_du_prix
                                .zip(reponse.interact_pointer_pos())
                                .is_some_and(|(r, p)| r.expand(2.0).contains(p));
                            let simple = !reponse.double_clicked() && !reponse.triple_clicked();
                            match &lien {
                                Some(l) if sur_le_prix && !ctrl && !maj => {
                                    if simple {
                                        a_ouvrir = Some(l.url.clone());
                                    }
                                }
                                _ => {
                                    clic = Some(i);
                                    // Le double clic ouvre la fiche ; son
                                    // premier clic a déjà fait de la ligne la
                                    // sélection.
                                    if reponse.double_clicked() && !ctrl && !maj {
                                        fiche_demandee = Some(adresse.clone());
                                    }
                                }
                            }
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
                            menu_selection(ui, n_selection.max(1), lien.as_ref(), &mut geste);
                        });
                    });
                });

            if let Some(url) = a_ouvrir {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            if let Some(adresse) = fiche_demandee {
                self.ouvrir_la_fiche(&adresse);
            }
            if let Some(adresse) = deplier {
                if !self.deplies.remove(&adresse) {
                    self.deplies.insert(adresse);
                }
            }
            if let Some((adresse, demande)) = demande_exemplaire {
                self.agir_sur_exemplaire(&adresse, &demande);
                return;
            }

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
            edition: String::new(),
            a_part: Vec::new(),
            overframe: false,
            card_image_id: Some(rowid),
            variantes: 1,
            variante: 0,
            prix: None,
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
    fn une_carte_depliee_montre_chacun_de_ses_exemplaires() {
        let mut cinq = carte("RA02", 1);
        cinq.quantite = 5;
        cinq.qualite = "NM".into();
        cinq.a_part = vec![(9, Etat::nouveau("PL", ""))];
        let mut une = carte("RA02", 2);
        une.quantite = 1;
        let visibles = vec![cinq, une];
        let mut deplies = BTreeSet::new();
        assert_eq!(
            rangees(&visibles, &deplies).len(),
            2,
            "replié : deux cartes"
        );

        deplies.insert(("RA02".to_owned(), 1));
        // Une carte d'un seul exemplaire ne se déplie pas, même demandée.
        deplies.insert(("RA02".to_owned(), 2));
        let r = rangees(&visibles, &deplies);
        assert_eq!(r.len(), 2 + 5, "les cinq exemplaires de la première");
        let a_part: Vec<usize> = r
            .iter()
            .filter_map(|x| match x {
                Rangee::Exemplaire {
                    numero,
                    exemplaire: Exemplaire::APart(_),
                    ..
                } => Some(*numero),
                _ => None,
            })
            .collect();
        assert_eq!(a_part, vec![5], "le réglé à part vient après les standard");
        assert!(matches!(r[6], Rangee::Carte(1)), "la seconde carte suit");
    }

    #[test]
    fn le_retour_se_consomme() {
        let mut e = ecran(Vec::new());
        e.retour = true;
        assert!(e.retour_demande());
        assert!(!e.retour_demande());
    }

    // ── La page Cardmarket d'une ligne (2026-11-05) ─────────────────────────

    /// Une offre sur la page d'un produit.
    fn offre_produit(id: i64) -> Offre {
        Offre {
            cible: Cible::Produit(id),
            version: Some("V.2 · Secret Rare".into()),
            prix: ygo_app::cardmarket::PrixOffre::Aucun,
            raison: ygo_app::cardmarket::Raison::Ordonnee,
            extension: None,
            prix_du: String::new(),
        }
    }

    /// Un écran de deux lignes chiffrées, chacune avec sa page : RA02-EN001
    /// à 3,88 € (deux exemplaires), RA02-EN002 à 1,50 € (trois).
    fn ecran_chiffre() -> EcranInventaire {
        let mut une = carte("RA02", 1);
        une.quantite = 2;
        une.prix = Some(388);
        let mut deux = carte("RA02", 2);
        deux.quantite = 3;
        deux.prix = Some(150);
        let mut e = ecran(vec![une, deux]);
        e.valorisation = Some(Valorisation {
            lignes: std::collections::HashMap::from([(
                "RA02".to_owned(),
                std::collections::HashMap::from([(1, Some(388)), (2, Some(150))]),
            )]),
            offres: std::collections::HashMap::from([(
                "RA02".to_owned(),
                std::collections::HashMap::from([
                    (1, offre_produit(782_049)),
                    (2, offre_produit(782_050)),
                ]),
            )]),
            annonces: std::collections::HashMap::from([(
                "RA02".to_owned(),
                LangueDesAnnonces::Francais,
            )]),
            ..Valorisation::default()
        });
        e
    }

    /// Le tableau de l'écran, joué image par image dans un contexte sans
    /// fenêtre — le vrai, pas une copie de sa logique.
    struct Banc {
        ctx: egui::Context,
        temps: f64,
        formes: Vec<egui::epaint::ClippedShape>,
    }

    impl Banc {
        fn nouveau() -> Self {
            Self {
                ctx: egui::Context::default(),
                temps: 0.0,
                formes: Vec::new(),
            }
        }

        /// Une image ; rend les adresses que l'écran a demandé d'ouvrir.
        fn image(
            &mut self,
            e: &mut EcranInventaire,
            evenements: Vec<egui::Event>,
            modificateurs: egui::Modifiers,
        ) -> Vec<String> {
            self.temps += 0.05;
            // Les touches tenues arrivent en événement, avant le reste.
            let mut tous = vec![egui::Event::ModifiersChanged(modificateurs)];
            tous.extend(evenements);
            let entree = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 500.0),
                )),
                time: Some(self.temps),
                events: tous,
                ..Default::default()
            };
            // L'ordre de `ui` : les raccourcis, le tableau, la fiche par-dessus.
            // Ni les prix (le fil du chiffrage rendrait `None` et effacerait
            // ceux du test), ni les barres, que les tests ne regardent pas.
            let mut sortie = self.ctx.run_ui(entree, |racine| {
                e.raccourcis(racine.ctx());
                e.table(racine);
                e.montrer_la_fiche(racine.ctx());
            });
            // Sans moteur de rendu, personne ne consomme les textures.
            sortie.textures_delta.clear();
            self.formes = sortie.shapes;
            sortie
                .platform_output
                .commands
                .into_iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::OpenUrl(o) => Some(o.url),
                    _ => None,
                })
                .collect()
        }

        /// Quelques images sans rien faire : les colonnes `auto` se mesurent
        /// à la première, et les textes ne bougent plus ensuite.
        fn poser(&mut self, e: &mut EcranInventaire) {
            for _ in 0..3 {
                self.image(e, Vec::new(), egui::Modifiers::NONE);
            }
        }

        /// Le rectangle d'un texte de la dernière image.
        fn rect_de(&self, texte: &str) -> egui::Rect {
            fn chercher(forme: &egui::Shape, texte: &str) -> Option<egui::Rect> {
                match forme {
                    egui::Shape::Text(t) if t.galley.text() == texte => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()))
                    }
                    egui::Shape::Vec(formes) => formes.iter().find_map(|f| chercher(f, texte)),
                    _ => None,
                }
            }
            self.formes
                .iter()
                .find_map(|c| chercher(&c.shape, texte))
                .unwrap_or_else(|| panic!("« {texte} » n'est pas dessiné"))
        }

        /// Un clic complet au centre d'un texte : rend les adresses ouvertes.
        fn cliquer(
            &mut self,
            e: &mut EcranInventaire,
            texte: &str,
            bouton: egui::PointerButton,
            modificateurs: egui::Modifiers,
        ) -> Vec<String> {
            let p = self.rect_de(texte).center();
            let appui = |pressed| egui::Event::PointerButton {
                pos: p,
                button: bouton,
                pressed,
                modifiers: modificateurs,
            };
            let mut ouvertes = Vec::new();
            for evenements in [
                vec![egui::Event::PointerMoved(p)],
                vec![appui(true)],
                vec![appui(false)],
                Vec::new(),
            ] {
                ouvertes.extend(self.image(e, evenements, modificateurs));
            }
            // Le clic suivant ne fera pas un double clic avec celui-ci.
            self.temps += 1.0;
            ouvertes
        }

        /// Un double clic au centre d'un texte — deux clics à un dixième de
        /// seconde : rend les adresses ouvertes.
        fn double_cliquer(
            &mut self,
            e: &mut EcranInventaire,
            texte: &str,
            modificateurs: egui::Modifiers,
        ) -> Vec<String> {
            let p = self.rect_de(texte).center();
            let appui = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: modificateurs,
            };
            let mut ouvertes = Vec::new();
            for evenements in [
                vec![egui::Event::PointerMoved(p)],
                vec![appui(true)],
                vec![appui(false)],
                vec![appui(true)],
                vec![appui(false)],
                Vec::new(),
            ] {
                ouvertes.extend(self.image(e, evenements, modificateurs));
            }
            self.temps += 1.0;
            ouvertes
        }

        /// Une touche enfoncée puis relâchée.
        fn touche(&mut self, e: &mut EcranInventaire, touche: egui::Key) {
            let evenement = |pressed| egui::Event::Key {
                key: touche,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.image(e, vec![evenement(true)], egui::Modifiers::NONE);
            self.image(e, vec![evenement(false)], egui::Modifiers::NONE);
        }
    }

    const PAGE_UNE: &str =
        "https://www.cardmarket.com/fr/YuGiOh/Products?idProduct=782049&language=2";

    /// Un clic sur le prix ouvre la page de la ligne — celle de sa fiche —
    /// sans toucher à la sélection ; un clic sur le nom sélectionne sans
    /// rien ouvrir ; `Ctrl+clic` sur un prix garde son sens de sélection.
    #[test]
    fn un_clic_sur_le_prix_ouvre_la_page_cardmarket() {
        let mut e = ecran_chiffre();
        let mut banc = Banc::nouveau();
        let rien = egui::Modifiers::NONE;
        let gauche = egui::PointerButton::Primary;
        banc.poser(&mut e);

        assert_eq!(banc.cliquer(&mut e, "3,88 €", gauche, rien), vec![PAGE_UNE]);
        assert!(e.selection.is_empty(), "ouvrir n'est pas sélectionner");

        // La valeur (2 × 3,88 €) n'est pas un lien : elle sélectionne.
        assert!(banc.cliquer(&mut e, "7,76 €", gauche, rien).is_empty());
        assert_eq!(e.selection.len(), 1);

        assert!(banc.cliquer(&mut e, "Card 2", gauche, rien).is_empty());
        assert!(e.selection.contains(&("RA02".to_owned(), 2)));

        // Ctrl+clic sur le prix de la première : ajoutée, pas ouverte.
        let ctrl = egui::Modifiers::COMMAND;
        assert!(banc.cliquer(&mut e, "3,88 €", gauche, ctrl).is_empty());
        assert_eq!(e.selection.len(), 2);
    }

    /// Le clic droit propose la page de la ligne cliquée, nommée par son
    /// numéro, et l'ouvre.
    #[test]
    fn le_clic_droit_propose_la_page_cardmarket() {
        let mut e = ecran_chiffre();
        let mut banc = Banc::nouveau();
        let rien = egui::Modifiers::NONE;
        banc.poser(&mut e);

        banc.cliquer(&mut e, "Card 1", egui::PointerButton::Secondary, rien);
        let entree = "Voir RA02-EN001 sur Cardmarket · V.2 Secret Rare";
        let ouvertes = banc.cliquer(&mut e, entree, egui::PointerButton::Primary, rien);
        assert_eq!(ouvertes, vec![PAGE_UNE]);
    }

    /// La bulle ne dit pas deux fois pourquoi le lien mène à une recherche,
    /// et donne la version à ouvrir quand on la connaît.
    #[test]
    fn l_aide_du_lien_ne_se_repete_pas() {
        let mut absente = offre_produit(1);
        absente.cible = Cible::Numero("CORI-JP067".into());
        absente.version = None;
        absente.raison = ygo_app::cardmarket::Raison::CarteAbsente;
        let lien = Lien {
            offre: &absente,
            set_code: "CORI-JP067",
            annonces: LangueDesAnnonces::Francais,
            url: String::new(),
        };
        let aide = lien.aide().to_lowercase();
        let raison = absente.raison.explication().to_lowercase();
        assert_eq!(aide.matches(raison.as_str()).count(), 1, "{aide}");

        let mut recherche = offre_produit(1);
        recherche.cible = Cible::Recherche {
            nom: "El Shaddoll Winda".into(),
            id_extension: Some(5989),
        };
        recherche.version = Some("V.6 · Quarter Century Secret Rare".into());
        let lien = Lien {
            offre: &recherche,
            set_code: "RA03-EN001",
            annonces: LangueDesAnnonces::Francais,
            url: String::new(),
        };
        assert!(lien
            .aide()
            .starts_with("Version à ouvrir : V.6 · Quarter Century Secret Rare\n"));
    }

    /// Une ligne possédée depuis le chiffrage n'a pas encore de page : son
    /// tiret reste un tiret, et un clic dessus sélectionne, comme avant.
    #[test]
    fn une_ligne_sans_offre_garde_son_tiret_ordinaire() {
        let mut e = ecran_chiffre();
        for c in &mut e.cartes {
            c.qualite = "NM".into();
        }
        e.cartes[1].prix = None;
        if let Some(v) = &mut e.valorisation {
            v.offres.get_mut("RA02").unwrap().remove(&2);
            v.lignes.get_mut("RA02").unwrap().remove(&2);
        }
        let mut banc = Banc::nouveau();
        let rien = egui::Modifiers::NONE;
        banc.poser(&mut e);
        assert!(banc
            .cliquer(&mut e, "—", egui::PointerButton::Primary, rien)
            .is_empty());
        assert!(e.selection.contains(&("RA02".to_owned(), 2)));
    }

    // ── La fiche, au double clic (2026-11-05) ───────────────────────────────

    /// Une installation de trois lignes possédées dans `RA02` : Carte A, B
    /// et C, aux numéros `RA02-EN001` à `003`. La base des cartes manque :
    /// la fiche s'ouvre quand même, sans texte.
    fn installation_de_trois() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (rowid, nom) in [(1_i64, "Carte A"), (2, "Carte B"), (3, "Carte C")] {
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite)
                 VALUES (?1, ?2, ?3, 'Common', 1, 1)",
                (rowid, nom, format!("RA02-EN00{rowid}")),
            )
            .unwrap();
        }
        drop(conn);
        (tmp, paths)
    }

    /// Le double clic ouvre la fiche de la ligne, qui devient la sélection ;
    /// avec `Ctrl`, il garde son sens de sélection et n'ouvre rien.
    #[test]
    fn un_double_clic_ouvre_la_fiche_de_la_ligne() {
        let (_tmp, paths) = installation_de_trois();
        let mut e = EcranInventaire::ouvrir(paths, true);
        let mut banc = Banc::nouveau();
        banc.poser(&mut e);

        banc.double_cliquer(&mut e, "Carte A", egui::Modifiers::COMMAND);
        assert!(
            !e.fiche.est_ouverte(),
            "Ctrl+double-clic : sélection seulement"
        );

        let ouvertes = banc.double_cliquer(&mut e, "Carte B", egui::Modifiers::NONE);
        assert!(ouvertes.is_empty(), "rien dans le navigateur");
        assert!(e.fiche.est_ouverte());
        assert_eq!(e.ligne_de_la_fiche, Some(("RA02".to_owned(), 2)));
        assert_eq!(
            e.fiche.carte().map(|f| f.set_code.as_str()),
            Some("RA02-EN002")
        );
        assert_eq!(e.selection, BTreeSet::from([("RA02".to_owned(), 2)]));
    }

    /// Un double clic sur le prix n'ouvre que la page Cardmarket — une fois
    /// —, pas la fiche par-dessus.
    #[test]
    fn un_double_clic_sur_le_prix_n_ouvre_que_cardmarket() {
        let (_tmp, paths) = installation_de_trois();
        let mut e = EcranInventaire::ouvrir(paths, true);
        e.valorisation = Some(Valorisation {
            lignes: std::collections::HashMap::from([(
                "RA02".to_owned(),
                std::collections::HashMap::from([(2, Some(388))]),
            )]),
            offres: std::collections::HashMap::from([(
                "RA02".to_owned(),
                std::collections::HashMap::from([(2, offre_produit(782_049))]),
            )]),
            ..Valorisation::default()
        });
        e.poser_les_prix();
        let mut banc = Banc::nouveau();
        banc.poser(&mut e);

        let ouvertes = banc.double_cliquer(&mut e, "3,88 €", egui::Modifiers::NONE);
        assert_eq!(ouvertes, vec![PAGE_UNE], "un seul onglet");
        assert!(!e.fiche.est_ouverte());
    }

    /// La fiche feuillette la liste affichée, sans boucler ; la ligne montrée
    /// devient la sélection et le tableau est prié de l'amener sous les
    /// yeux. Une relecture la tient à jour, et la ferme si sa ligne a quitté
    /// l'inventaire.
    #[test]
    fn la_fiche_feuillette_la_liste_affichee() {
        let (_tmp, paths) = installation_de_trois();
        let mut e = EcranInventaire::ouvrir(paths.clone(), true);
        let ligne = |rowid: i64| ("RA02".to_owned(), rowid);
        e.ouvrir_la_fiche(&ligne(2));

        e.deplacer_la_fiche(1);
        assert_eq!(e.ligne_de_la_fiche, Some(ligne(3)));
        assert_eq!(e.selection, BTreeSet::from([ligne(3)]));
        assert_eq!(e.a_montrer, Some(ligne(3)));
        e.deplacer_la_fiche(1);
        assert_eq!(e.ligne_de_la_fiche, Some(ligne(3)), "pas de bouclage");

        // Trié par nom descendant, la voisine « suivante » de C est B.
        e.colonne = Colonne::Nom;
        e.descendant = true;
        e.trier();
        e.deplacer_la_fiche(1);
        assert_eq!(e.ligne_de_la_fiche, Some(ligne(2)), "l'ordre de l'écran");

        // La quantité change : la fiche se relit.
        ygo_app::inventaire::definir_quantite(&paths, &[ligne(2)], 3).unwrap();
        e.relire();
        assert!(e.fiche.est_ouverte());
        let possession = e.fiche.carte().unwrap().possession();
        assert!(possession.starts_with("×3"), "{possession}");
        // Retirée de l'inventaire : la fiche se ferme.
        ygo_app::inventaire::retirer(&paths, &[ligne(2)]).unwrap();
        e.relire();
        assert!(!e.fiche.est_ouverte());
        assert_eq!(e.ligne_de_la_fiche, None);
    }

    /// Fiche ouverte, `Échap` la ferme et ne désélectionne rien ; fermée, il
    /// désélectionne de nouveau.
    #[test]
    fn echap_ferme_la_fiche_sans_vider_la_selection() {
        let (_tmp, paths) = installation_de_trois();
        let mut e = EcranInventaire::ouvrir(paths, true);
        let mut banc = Banc::nouveau();
        banc.poser(&mut e);
        banc.double_cliquer(&mut e, "Carte B", egui::Modifiers::NONE);
        assert!(e.fiche.est_ouverte());

        banc.touche(&mut e, egui::Key::Escape);
        assert!(!e.fiche.est_ouverte(), "Échap ferme la fiche");
        assert_eq!(e.selection.len(), 1, "… sans désélectionner");

        banc.touche(&mut e, egui::Key::Escape);
        assert!(e.selection.is_empty(), "fiche fermée, Échap désélectionne");
    }
}
