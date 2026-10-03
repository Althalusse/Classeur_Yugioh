// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les données de l'écran classeur — tout sauf les pixels.
//!
//! Portage de la part calculatoire d'`ui/ecran_classeur.py` (1 726 l.) et de
//! `carte_posseder/affichage_carte_classeur.get_cartes_info`.
//!
//! # Un classeur, pas une grille
//!
//! L'écran n'est pas une liste défilante : c'est un **classeur en double
//! page**. Le nombre de cases par page vient de la configuration du classeur —
//! 3×3 par défaut, modifiable par classeur —, et c'est elle qui détermine tout
//! le reste.
//!
//! ## Invariant : la première page est TOUJOURS seule
//!
//! Règle immuable, posée par l'auteur du projet et reprise du Python : le
//! premier feuillet ne montre **qu'une** page, à droite, comme quand on ouvre
//! un vrai album — la couverture occupe la gauche. Les feuillets suivants
//! montrent deux pages, `2s` et `2s + 1`.
//!
//! Elle n'est écrite qu'à **un seul endroit**, le cas `indice == 0` de
//! [`double_page`], et [`nb_doubles_pages`] en découle. Trois tests la
//! gardent — `la_premiere_double_page_est_seule_a_droite`,
//! `l_invariant_de_premiere_page_vaut_pour_toutes_les_grilles`, et le test
//! d'oracle sur les 26 classeurs réels — et trois mutations vérifient qu'aucun
//! moyen de la violer ne passe : remplir les deux côtés, mettre la page à
//! gauche, ou supprimer le cas particulier.
//!
//! Conséquence sur laquelle il vaut mieux ne pas se tromper : à l'écran il n'y
//! a jamais plus de `2 × colonnes × lignes` cartes, **dix-huit** dans la
//! configuration par défaut, quel que soit le nombre de cartes du classeur.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne dessine rien, ne décode aucune image et ne connaît aucun type
//! d'interface. C'est ce qui permet d'en éprouver chaque règle contre le
//! Python, là où l'écran lui-même n'aura jamais d'oracle.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use ygo_core::config::{CritereTri, SourceImage, GRILLE_MAX, GRILLE_MIN};
use ygo_core::rarity::Priorites;
use ygo_core::tri::CarteTriable;

use crate::error::Result;

/// Rapport largeur/hauteur d'une carte Yu-Gi-Oh!.
pub const RATIO_LARGEUR: u32 = 59;
/// Rapport largeur/hauteur d'une carte Yu-Gi-Oh!.
pub const RATIO_HAUTEUR: u32 = 86;

/// Une carte du classeur, telle que l'écran la consomme.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Carte {
    /// Identifiant de la ligne — vise une carte sans ambiguïté.
    pub rowid: i64,
    /// Nom **résolu** dans la langue courante.
    pub nom: String,
    /// Libellé de rareté.
    pub rarete: String,
    /// Code de set complet.
    pub set_code: String,
    /// Nom du set.
    pub set_name: String,
    /// Identifiant d'illustration ; négatif pour une image externe.
    pub card_image_id: i64,
    /// Nom de fichier de l'image, déduit de l'URL retenue.
    pub fichier_image: Option<String>,
    /// La carte est-elle marquée possédée ?
    pub possedee: bool,
    /// Quantité possédée, jamais négative.
    pub quantite: i64,
    /// Rang de tri d'origine.
    pub sort_order: i64,
    /// Ajout manuel de l'utilisateur.
    pub is_custom: bool,
    /// Overframe / art étendu.
    pub extended_art: bool,
}

impl CarteTriable for Carte {
    fn nom(&self) -> &str {
        &self.nom
    }
    fn set_code(&self) -> &str {
        &self.set_code
    }
    fn rarete(&self) -> &str {
        &self.rarete
    }
    fn card_image_id(&self) -> i64 {
        self.card_image_id
    }
    fn extended_art(&self) -> bool {
        self.extended_art
    }
}

/// Le nom de fichier d'une URL d'image.
///
/// Portage de `get_image_filename_from_url` : le *basename* du chemin de
/// l'URL, sans la requête ni le fragment.
#[must_use]
pub fn fichier_depuis_url(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    // On isole le chemin comme le ferait `urlparse().path`.
    let sans_schema = url.split("://").last().unwrap_or(url);
    let chemin = sans_schema
        .split_once('/')
        .map_or("", |(_hote, reste)| reste);
    let chemin = chemin.split(['?', '#']).next().unwrap_or_default();
    let base = chemin.rsplit('/').next().unwrap_or_default();
    (!base.is_empty()).then(|| base.to_owned())
}

/// Charge les cartes d'un classeur, triées comme l'utilisateur le demande.
///
/// Portage de `get_cartes_info`, suivi de `sort_cartes` — ce dernier étant
/// déjà [`ygo_core::tri::trier`].
///
/// # Les colonnes qui peuvent manquer
///
/// `is_custom` et `extended_art` sont absentes des classeurs de première
/// génération. Le Python construit sa requête en conséquence ; le portage fait
/// de même, plutôt que d'échouer sur une base ancienne.
///
/// # Le critère d'affichage
///
/// Une carte s'affiche si elle a **une image** — URL ou identifiant — ou si
/// c'est un **ajout manuel**. Ce dernier cas couvre les artworks que
/// l'utilisateur a ajoutés volontairement sans image.
pub fn charger(
    conn: &Connection,
    francais: bool,
    source: SourceImage,
    ordre: [CritereTri; 3],
    priorites: &Priorites,
) -> Result<Vec<Carte>> {
    let colonnes = colonnes_presentes(conn)?;
    let a_custom = colonnes.iter().any(|c| c == "is_custom");
    let a_ext = colonnes.iter().any(|c| c == "extended_art");
    let a_nom_fr = colonnes.iter().any(|c| c == "name_fr");

    // `COALESCE(NULLIF(name_fr,''), name)` : un nom français vide retombe sur
    // l'anglais. Sur un classeur sans la colonne, le français n'existe pas.
    let expression_nom = if francais && a_nom_fr {
        "COALESCE(NULLIF(name_fr, ''), name)"
    } else {
        "name"
    };
    let expression_custom = if a_custom {
        "COALESCE(is_custom, 0)"
    } else {
        "0"
    };
    let expression_ext = if a_ext {
        "COALESCE(extended_art, 0)"
    } else {
        "0"
    };
    let clause_custom = if a_custom { " OR is_custom = 1" } else { "" };

    let sql = format!(
        "SELECT rowid, card_image_url, {expression_nom}, rarity, set_code, \
                possessed, set_name, card_image_id, COALESCE(quantite, 0), \
                COALESCE(sort_order, 0), {expression_custom}, {expression_ext} \
         FROM cards \
         WHERE card_image_url IS NOT NULL OR card_image_id IS NOT NULL{clause_custom} \
         ORDER BY sort_order, rarity"
    );

    let mut requete = conn.prepare(&sql)?;
    let cartes: Vec<Carte> = requete
        .query_map([], |l| {
            let url: Option<String> = l.get(1)?;
            let card_image_id: Option<i64> = l.get(7)?;
            let retenue = ygo_core::image_source::url_image(source, url.as_deref(), card_image_id);
            Ok(Carte {
                rowid: l.get(0)?,
                fichier_image: retenue.as_deref().and_then(fichier_depuis_url),
                nom: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                rarete: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
                set_code: l.get::<_, Option<String>>(4)?.unwrap_or_default(),
                possedee: l.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                set_name: l.get::<_, Option<String>>(6)?.unwrap_or_default(),
                card_image_id: card_image_id.unwrap_or(0),
                quantite: l.get::<_, Option<i64>>(8)?.unwrap_or(0).max(0),
                sort_order: l.get::<_, Option<i64>>(9)?.unwrap_or(0),
                is_custom: l.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
                extended_art: l.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    Ok(ygo_core::tri::trier(cartes, ordre, priorites))
}

/// Les colonnes de la table `cards`.
fn colonnes_presentes(conn: &Connection) -> Result<Vec<String>> {
    let mut requete = conn.prepare("SELECT name FROM pragma_table_info('cards')")?;
    let noms: Vec<String> = requete
        .query_map([], |l| l.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(noms)
}

// ─────────────────────────────────────────────────────────────────────────────
// Filtres
// ─────────────────────────────────────────────────────────────────────────────

/// Le filtre de possession de la barre de recherche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Possession {
    /// Aucun filtre.
    #[default]
    Toutes,
    /// Seulement les cartes dont la **quantité** est positive.
    Possedees,
    /// Seulement celles dont la quantité est nulle.
    NonPossedees,
}

/// Ce que l'utilisateur a saisi dans la barre de recherche.
#[derive(Debug, Clone, Default)]
pub struct Filtres {
    /// Terme cherché dans le nom **ou** le code de set.
    pub terme: String,
    /// Rareté exacte, ou `None` pour « Toutes ».
    pub rarete: Option<String>,
    /// Filtre de possession.
    pub possession: Possession,
    /// Nombre de raretés gardées par artwork ; `0` les garde toutes.
    ///
    /// Résolution côté appelant : un réglage propre au classeur écrase le
    /// réglage global, **y compris `0`**, qui force « toutes les raretés ».
    pub n_raretes: usize,
}

/// Réduit un texte à ses caractères alphanumériques, en minuscules.
///
/// Sert à faire correspondre un numéro de collection saisi sans séparateur :
/// `ra02en008` retrouve `RA02-EN008`.
#[must_use]
pub fn compacter(texte: &str) -> String {
    texte
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// La carte répond-elle au terme cherché ?
///
/// # Deux passes, et la première est celle du Python
///
/// La recherche du Python est une simple sous-chaîne sur le nom **ou** le code
/// de set, tous deux en minuscules : `080` retrouve `RA02-EN080`, `-EN0`
/// retrouve tout le set. On la garde telle quelle — les oracles la figent,
/// `-EN0` compris, et elle dépend du tiret.
///
/// La seconde passe est un **ajout** : le code de set compacté, séparateurs
/// ôtés, pour que `ra02en008` retrouve `RA02-EN008`. Étant une disjonction,
/// elle ne peut qu'ajouter des correspondances — jamais en retirer, donc
/// jamais faire mentir un oracle.
///
/// Ce qu'elle ne fait **pas** : deviner un numéro absent. Un terme qui ne
/// ramène rien ne ramène rien parce que le classeur n'a pas la carte ; c'est à
/// l'écran de le dire, pas au filtre de l'inventer.
#[must_use]
fn correspond(c: &Carte, terme: &str, terme_compact: &str) -> bool {
    if c.nom.to_lowercase().contains(terme) || c.set_code.to_lowercase().contains(terme) {
        return true;
    }
    !terme_compact.is_empty() && compacter(&c.set_code).contains(terme_compact)
}

/// Applique les filtres, dans l'ordre du Python.
///
/// # L'ordre n'est pas indifférent
///
/// Le filtre « N raretés par artwork » vient **en dernier**, après la
/// recherche, la rareté et la possession. C'est délibéré : la rareté gagnante
/// de chaque groupe est ainsi choisie **parmi ce qui reste**. Si l'utilisateur
/// a filtré sur « Possédées », il veut la plus rare de *ses* cartes, pas la
/// plus rare dans l'absolu — qu'il pourrait ne pas posséder.
///
/// La possession se lit sur la **quantité**, pas sur le drapeau `possessed` :
/// c'est ce que fait le Python, et les deux peuvent diverger.
#[must_use]
pub fn appliquer(cartes: Vec<Carte>, filtres: &Filtres, priorites: &Priorites) -> Vec<Carte> {
    let terme = filtres.terme.trim().to_lowercase();
    let terme_compact = compacter(&terme);
    let retenues: Vec<Carte> = cartes
        .into_iter()
        .filter(|c| {
            if !terme.is_empty() && !correspond(c, &terme, &terme_compact) {
                return false;
            }
            if let Some(rarete) = &filtres.rarete {
                if &c.rarete != rarete {
                    return false;
                }
            }
            match filtres.possession {
                Possession::Toutes => true,
                Possession::Possedees => c.quantite > 0,
                Possession::NonPossedees => c.quantite == 0,
            }
        })
        .collect();

    if filtres.n_raretes > 0 {
        ygo_core::tri::filtrer_n_raretes_par_artwork(retenues, filtres.n_raretes, priorites)
    } else {
        retenues
    }
}

/// Les raretés proposées par la liste déroulante, triées.
#[must_use]
pub fn raretes_disponibles(cartes: &[Carte]) -> Vec<String> {
    let mut raretes: Vec<String> = cartes
        .iter()
        .filter(|c| !c.rarete.is_empty())
        .map(|c| c.rarete.clone())
        .collect();
    raretes.sort();
    raretes.dedup();
    raretes
}

// ─────────────────────────────────────────────────────────────────────────────
// Pagination — le classeur en double page
// ─────────────────────────────────────────────────────────────────────────────

/// La configuration de grille d'un classeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grille {
    /// Colonnes par page.
    pub colonnes: u8,
    /// Lignes par page.
    pub lignes: u8,
}

impl Grille {
    /// Cases par page.
    #[must_use]
    pub fn par_page(self) -> usize {
        usize::from(self.colonnes) * usize::from(self.lignes)
    }
}

/// Nombre total de doubles pages.
///
/// Portage de `_nb_spreads`. La première page étant **seule** à droite, les
/// suivantes vont par deux :
///
/// - aucune carte → **1** double page, vide ;
/// - une seule page → **1** ;
/// - `n` pages → `1 + ⌈(n − 1) / 2⌉`.
#[must_use]
pub fn nb_doubles_pages(nb_cartes: usize, grille: Grille) -> usize {
    let par_page = grille.par_page();
    if nb_cartes == 0 || par_page == 0 {
        return 1;
    }
    let pages = nb_cartes.div_ceil(par_page);
    if pages <= 1 {
        return 1;
    }
    1 + (pages - 1).div_ceil(2)
}

/// Les cartes des deux pages d'une double page.
///
/// Portage de `_spread_slices` :
///
/// - double page **0** : gauche vide, droite = page 0 ;
/// - double page **s** : gauche = page `2s − 1`, droite = page `2s`.
///
/// Une tranche hors des cartes disponibles rend simplement une page vide —
/// c'est le comportement du découpage Python, qui ne lève jamais.
#[must_use]
pub fn double_page(cartes: &[Carte], indice: usize, grille: Grille) -> (&[Carte], &[Carte]) {
    let par_page = grille.par_page();
    if par_page == 0 {
        return (&[], &[]);
    }
    if indice == 0 {
        return (&[], tranche(cartes, 0, par_page));
    }
    let gauche = tranche(cartes, (2 * indice - 1) * par_page, par_page);
    let droite = tranche(cartes, (2 * indice) * par_page, par_page);
    (gauche, droite)
}

/// Une tranche bornée, comme le découpage Python : jamais d'erreur.
fn tranche(cartes: &[Carte], debut: usize, longueur: usize) -> &[Carte] {
    if debut >= cartes.len() {
        return &[];
    }
    let fin = debut.saturating_add(longueur).min(cartes.len());
    cartes.get(debut..fin).unwrap_or(&[])
}

/// Numéro de page affiché pour une double page, tel que l'utilisateur le lit.
///
/// La première double page ne montre que la page 1. Les suivantes montrent
/// `2s` et `2s + 1`.
#[must_use]
pub fn numeros_de_page(indice: usize) -> (Option<usize>, usize) {
    if indice == 0 {
        return (None, 1);
    }
    (Some(2 * indice), 2 * indice + 1)
}

/// Le feuillet qui montre une page donnée — l'inverse de [`numeros_de_page`].
///
/// # Une division qui tombe juste, et une garde qui ne servait à rien
///
/// Un classeur ne s'ouvre pas sur deux pages : le premier feuillet ne montre
/// que la page 1, à droite, comme un vrai classeur qu'on ouvre. La
/// correspondance est donc décalée :
///
/// | page | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
/// |---|---|---|---|---|---|---|---|
/// | feuillet | 0 | 1 | 1 | 2 | 2 | 3 | 3 |
///
/// Ce décalage m'a fait écrire un cas particulier pour les pages 0 et 1. Deux
/// mutations y ont survécu, et pour cause : la division entière rend déjà zéro
/// pour l'une comme pour l'autre. La garde était du décor. Ce qui reste est la
/// **borne haute** — une page demandée au-delà du classeur mène à sa fin
/// plutôt qu'à un feuillet qui n'existe pas.
///
/// ```
/// use ygo_app::classeur::feuillet_de_page;
/// assert_eq!(feuillet_de_page(1, 19), 0);
/// assert_eq!(feuillet_de_page(7, 19), 3, "le feuillet 3 montre les pages 6 et 7");
/// assert_eq!(feuillet_de_page(0, 19), 0, "il n'y a pas de page 0");
/// assert_eq!(feuillet_de_page(999, 19), 18, "borné au dernier feuillet");
/// ```
#[must_use]
pub fn feuillet_de_page(page: usize, feuillets: usize) -> usize {
    (page / 2).min(feuillets.saturating_sub(1))
}

/// Le numéro de page le plus élevé qu'un classeur porte.
///
/// Sert à borner la saisie : proposer d'aller à la page 80 d'un classeur qui
/// en compte 37 n'aide personne.
///
/// ```
/// use ygo_app::classeur::derniere_page;
/// assert_eq!(derniere_page(1), 1, "un seul feuillet, une seule page");
/// assert_eq!(derniere_page(19), 37);
/// assert_eq!(derniere_page(0), 1, "un classeur vide en montre une quand même");
/// ```
#[must_use]
pub fn derniere_page(feuillets: usize) -> usize {
    if feuillets <= 1 {
        return 1;
    }
    2 * (feuillets - 1) + 1
}

// ─────────────────────────────────────────────────────────────────────────────
// Taille adaptative des cartes
// ─────────────────────────────────────────────────────────────────────────────

/// Marge extérieure du spread, en points (`_OUTER_PAD`).
pub const MARGE_EXTERIEURE: u32 = 40;
/// Réserve pour la barre de défilement (`_SCROLLBAR_W`).
pub const RESERVE_DEFILEMENT: u32 = 20;
/// Espace entre les deux pages (`_SPREAD_GAP`).
pub const ESPACE_ENTRE_PAGES: u32 = 24;
/// Padding intérieur d'une page (`_PAGE_INNER_PAD`).
pub const PADDING_PAGE: u32 = 16;
/// Espace entre deux cartes (`BinderPage.GAP`).
pub const ESPACE_CARTES: u32 = 6;
/// Marge au-dessus et au-dessous de la double page.
///
/// Sans équivalent Python : le calcul d'origine ignorait la hauteur, il
/// n'avait donc rien à en retrancher.
///
/// # Sa valeur est libre, son unicité ne l'est pas
///
/// Douze points ou zéro, la mise en page reste juste — c'est un réglage de
/// respiration. Ce qui n'est pas négociable, c'est que [`taille_carte`] et
/// l'écran emploient la **même** : le calcul retranche cette marge pour
/// décider de la taille des cartes, et si le dessin en ajoutait une autre,
/// la double page déborderait exactement de la différence. D'où la
/// constante partagée plutôt que deux littéraux qui se ressemblent.
///
/// Une mutation qui la met à zéro survit donc à tous les tests, et c'est
/// correct : elle ne casse rien tant qu'il n'y a qu'une source.
pub const MARGE_VERTICALE: u32 = 12;

/// La zone d'affichage disponible, en points.
///
/// Deux entiers de même type côte à côte dans une signature s'échangent sans
/// que rien ne proteste ; les nommer rend l'inversion impossible à écrire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fenetre {
    /// Largeur utile.
    pub largeur: u32,
    /// Hauteur utile.
    pub hauteur: u32,
}

/// Bornes de largeur d'une carte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BornesCarte {
    /// Largeur minimale.
    pub min: u32,
    /// Largeur maximale.
    pub max: u32,
}

/// Taille d'une carte pour une fenêtre donnée.
///
/// Portage de `_compute_card_size`, **avec une divergence assumée**. Le
/// raisonnement du Python : remplir la largeur avec deux pages côte à côte,
/// chacune portant sa grille, puis borner pour rester lisible. La hauteur
/// suit le ratio 59:86 d'une carte Yu-Gi-Oh!.
///
/// # Ce qui change, et pourquoi
///
/// Le Python ne regardait que la **largeur**. Sur un écran large, ça donne
/// deux torts à la fois :
///
/// - la borne haute est atteinte tôt (220 points dès 1 700 de fenêtre en
///   3×3) et tout le reste devient du noir — sur 2 560, la double page
///   occupait 1 456 points et laissait 1 100 de vide ;
/// - si on relève simplement la borne, la largeur réclame 387 points par
///   carte, donc 1 692 points de hauteur pour trois lignes. Plus que ce
///   qu'un écran 1440p offre : la double page passerait sous la barre de
///   défilement, ce qui est pire que trop petit.
///
/// On prend donc le **minimum des deux contraintes**. Sur 2 560 × 1 440 en
/// 3×3, la largeur autoriserait 387 et la hauteur 300 : c'est 300.
///
/// # Ce que la borne haute veut dire
///
/// Elle n'est plus là pour économiser du calcul — le Python refabriquait un
/// bitmap par taille avec Pillow, egui garde une texture unique que le GPU
/// redimensionne au dessin. Elle marque le point où l'image serait
/// **agrandie au-delà de sa définition**, donc floue. Mesure sur
/// l'installation réelle : 1 872 des 1 878 images du cache font 813 × 1 185
/// pixels. Autant dire qu'à moins d'un écran 8K, c'est l'ajustement qui
/// décide et jamais la borne.
///
/// Une dimension absurde (fenêtre pas encore affichée) retombe sur
/// 1 280 × 800 — le Python fait pareil pour la largeur quand
/// `winfo_width()` rend 1.
#[must_use]
pub fn taille_carte(fenetre: Fenetre, grille: Grille, bornes: BornesCarte) -> (u32, u32) {
    let largeur = if fenetre.largeur <= 1 {
        1280
    } else {
        fenetre.largeur
    };
    let hauteur = if fenetre.hauteur <= 1 {
        800
    } else {
        fenetre.hauteur
    };
    let colonnes = u32::from(grille.colonnes).max(1);
    let lignes = u32::from(grille.lignes).max(1);

    // Ce que la largeur permet : deux pages côte à côte.
    let disponible = largeur
        .saturating_sub(2 * MARGE_EXTERIEURE)
        .saturating_sub(RESERVE_DEFILEMENT);
    let par_page = disponible.saturating_sub(ESPACE_ENTRE_PAGES) / 2;
    let par_largeur = par_page
        .saturating_sub(2 * PADDING_PAGE)
        .saturating_sub((colonnes + 1) * ESPACE_CARTES)
        / colonnes;

    // Ce que la hauteur permet : une seule page, les deux ont la même.
    let haute = hauteur
        .saturating_sub(2 * MARGE_VERTICALE)
        .saturating_sub(2 * PADDING_PAGE)
        .saturating_sub((lignes + 1) * ESPACE_CARTES)
        / lignes;
    let par_hauteur = haute * RATIO_LARGEUR / RATIO_HAUTEUR;

    let largeur_carte = par_largeur.min(par_hauteur).clamp(bornes.min, bornes.max);
    let hauteur_carte = largeur_carte * RATIO_HAUTEUR / RATIO_LARGEUR;
    (largeur_carte, hauteur_carte)
}

/// Largeur occupée par les **deux pages** côte à côte, cartes bornées comprises.
///
/// # À quoi elle sert : centrer le classeur
///
/// Une double page 3×3 a un rapport largeur/hauteur d'environ 1,37 ; un écran
/// 16:9 en a 1,78. Sur un tel écran c'est donc la **hauteur** qui limite, et
/// la largeur ne sera jamais remplie : l'espace en trop s'accumulerait à
/// droite et le classeur resterait collé à gauche. Connaître la largeur
/// réellement occupée permet de répartir ce reste des deux côtés.
///
/// Le calcul suit exactement celui de [`taille_carte`], à l'envers : chaque
/// page porte `colonnes` cartes, `colonnes + 1` espaces entre elles, et deux
/// fois le rembourrage du cadre ; les deux pages sont séparées par
/// [`ESPACE_ENTRE_PAGES`].
///
/// ```
/// use ygo_app::classeur::{largeur_double_page, taille_carte, BornesCarte, Fenetre, Grille};
///
/// let grille = Grille { colonnes: 3, lignes: 3 };
/// let bornes = BornesCarte { min: 90, max: 813 };
///
/// // Fenêtre presque carrée : la largeur est le facteur limitant, le
/// // classeur occupe presque toute la largeur utile.
/// let (l, _) = taille_carte(Fenetre { largeur: 1280, hauteur: 1280 }, grille, bornes);
/// assert!(largeur_double_page(l, grille) <= 1280);
///
/// // Écran 16:9 : la hauteur limite, il reste de la largeur à répartir.
/// let (l, _) = taille_carte(Fenetre { largeur: 2560, hauteur: 1440 }, grille, bornes);
/// assert!(l < bornes.max, "ce n'est pas la borne qui limite");
/// assert!(largeur_double_page(l, grille) < 2100, "il reste à répartir");
/// ```
#[must_use]
pub fn largeur_double_page(largeur_carte: u32, grille: Grille) -> u32 {
    let colonnes = u32::from(grille.colonnes).max(1);
    let page = colonnes * largeur_carte + (colonnes + 1) * ESPACE_CARTES + 2 * PADDING_PAGE;
    2 * page + ESPACE_ENTRE_PAGES
}

/// Hauteur occupée par la double page, marges verticales comprises.
///
/// Le pendant de [`largeur_double_page`] pour l'autre axe. Les deux pages
/// ont la même hauteur, il n'y en a donc qu'une à compter.
///
/// ```
/// use ygo_app::classeur::{hauteur_double_page, taille_carte, BornesCarte, Fenetre, Grille};
///
/// let grille = Grille { colonnes: 3, lignes: 3 };
/// let bornes = BornesCarte { min: 90, max: 813 };
/// let fenetre = Fenetre { largeur: 2560, hauteur: 1440 };
///
/// let (_, h) = taille_carte(fenetre, grille, bornes);
/// assert!(hauteur_double_page(h, grille) <= fenetre.hauteur, "ça tient dans la fenêtre");
/// ```
#[must_use]
pub fn hauteur_double_page(hauteur_carte: u32, grille: Grille) -> u32 {
    let lignes = u32::from(grille.lignes).max(1);
    lignes * hauteur_carte + (lignes + 1) * ESPACE_CARTES + 2 * PADDING_PAGE + 2 * MARGE_VERTICALE
}

/// Le feuillet où se trouve la carte de rang `position`.
///
/// # L'invariant de la première page, encore lui
///
/// La page 0 est seule sur le feuillet 0 ; ensuite les pages vont par deux —
/// feuillet `s` porte les pages `2s − 1` et `2s`. Le rang se convertit donc en
/// deux temps : d'abord en numéro de page, puis en feuillet. Écrire cette
/// conversion ici plutôt qu'à l'écran, c'est la même règle qu'à
/// [`double_page`], au même endroit, et la garantie qu'elles ne divergeront
/// pas.
#[must_use]
pub fn feuillet_de(position: usize, grille: Grille) -> usize {
    let par_page = grille.par_page();
    if par_page == 0 {
        return 0;
    }
    // `div_ceil` couvre déjà la page 0 — elle rend 0. Une garde explicite
    // `if page == 0` y a d'abord figuré ; une mutation qui la supprimait n'a
    // fait échouer aucun test, pour la bonne raison qu'elle ne servait à rien.
    (position / par_page).div_ceil(2)
}

/// Une carte que la recherche propose, avec le feuillet où la trouver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// `rowid` de la carte.
    pub rowid: i64,
    /// Rang de la carte dans la liste affichée — d'où se déduit le feuillet.
    pub position: usize,
    /// Feuillet où elle se trouve, à partir de 0.
    pub feuillet: usize,
    /// Nom de la carte.
    pub nom: String,
    /// Numéro de collection.
    pub set_code: String,
    /// Rareté.
    pub rarete: String,
}

/// Nombre de propositions montrées sous la recherche.
pub const MAX_SUGGESTIONS: usize = 12;

/// Le numéro de collection d'un code de set — les chiffres de la fin.
///
/// `RA02-EN008` rend `8`, `LDK2-ENJ01` rend `1`. Les zéros de tête
/// disparaissent, ce qui est exactement ce qu'il faut : l'utilisateur tape
/// `8`, pas `008`. Un code sans chiffre final rend `None`.
#[must_use]
pub fn numero_de_collection(set_code: &str) -> Option<u32> {
    let chiffres: String = set_code
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    chiffres.parse().ok()
}

/// Les cartes que le terme cherché propose, les plus pertinentes d'abord.
///
/// # Le classement, et pourquoi il n'est pas alphabétique
///
/// Trois rangs, dans cet ordre :
///
/// 1. le **numéro exact** — le code entier (`RA02-EN008`, avec ou sans
///    séparateur) ou le seul numéro de collection (`8`, `008`) ;
/// 2. un nom qui **commence** par le terme — `dark` propose *Dark Magician*
///    avant *Buster Blader, the Dragon Destroyer Swordsman* ;
/// 3. le reste des correspondances.
///
/// À rang égal, l'ordre du classeur est conservé : les propositions suivent
/// l'ordre des pages, ce qui est le seul ordre que l'utilisateur a sous les
/// yeux.
///
/// Les doublons de rareté sont **écartés** : un numéro qui existe en sept
/// raretés ne doit pas remplir la liste à lui seul. C'est la première ligne
/// rencontrée qui représente le numéro — donc la plus proche du début du
/// classeur.
///
/// # Le rang 0 a failli ne rien faire
///
/// Il n'acceptait d'abord que le code **entier**. Or personne ne tape
/// `RA02-EN008` : on tape `8`. Et un terme aussi court était traité comme une
/// correspondance quelconque, derrière toutes les cartes dont le nom contient
/// un `8`. Une mutation qui désactivait le rang 0 ne faisait échouer aucun
/// test — la preuve que le rang ne servait à rien. Il compare désormais aussi
/// le **numéro de collection**, chiffres à chiffres, zéros de tête ignorés.
#[must_use]
pub fn suggestions(cartes: &[Carte], terme: &str, grille: Grille, max: usize) -> Vec<Suggestion> {
    let terme = terme.trim().to_lowercase();
    if terme.is_empty() {
        return Vec::new();
    }
    let compact = compacter(&terme);

    // Un terme entièrement numérique désigne un numéro de collection.
    let numero_cherche: Option<u32> = if terme.chars().all(|c| c.is_ascii_digit()) {
        terme.parse().ok()
    } else {
        None
    };

    let mut trouvees: Vec<(u8, usize, &Carte)> = Vec::new();
    let mut numeros_vus: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for (position, carte) in cartes.iter().enumerate() {
        let code = carte.set_code.to_lowercase();
        let nom = carte.nom.to_lowercase();
        let rang = if code == terme
            || compacter(&code) == compact
            || (numero_cherche.is_some() && numero_cherche == numero_de_collection(&carte.set_code))
        {
            0
        } else if nom.starts_with(&terme) {
            1
        } else if nom.contains(&terme)
            || code.contains(&terme)
            || (!compact.is_empty() && compacter(&code).contains(&compact))
        {
            2
        } else {
            continue;
        };
        if !numeros_vus.insert(carte.set_code.as_str()) {
            continue;
        }
        trouvees.push((rang, position, carte));
    }

    trouvees.sort_by_key(|(rang, position, _)| (*rang, *position));
    trouvees
        .into_iter()
        .take(max)
        .map(|(_, position, carte)| Suggestion {
            rowid: carte.rowid,
            position,
            feuillet: feuillet_de(position, grille),
            nom: carte.nom.clone(),
            set_code: carte.set_code.clone(),
            rarete: carte.rarete.clone(),
        })
        .collect()
}

/// La grille d'un classeur, lue dans sa table `meta`.
///
/// Réutilise [`crate::accueil::meta_classeur`] : c'est la même lecture, et
/// deux implémentations finiraient par diverger.
#[must_use]
pub fn grille_du_classeur(chemin_db: &Path, defaut: (u8, u8)) -> Grille {
    let meta = crate::accueil::meta_classeur(chemin_db, defaut);
    Grille {
        colonnes: meta.colonnes,
        lignes: meta.lignes,
    }
}

/// Les réglages qu'un classeur porte en propre.
///
/// `None` sur un champ veut dire **suivre les Options** : le classeur n'a rien
/// décidé, il prend le défaut de l'installation. C'est l'absence de la clé en
/// base qui le dit, pas une valeur sentinelle — un classeur créé avant que ces
/// réglages existent se comporte donc exactement comme avant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reglages {
    /// La grille propre au classeur.
    pub grille: Option<Grille>,
    /// L'ordre de tri propre au classeur.
    pub ordre: Option<[CritereTri; 3]>,
}

impl Reglages {
    /// Le classeur suit-il les Options en tout ?
    #[must_use]
    pub fn suit_les_options(&self) -> bool {
        self.grille.is_none() && self.ordre.is_none()
    }
}

/// La clé de `meta` qui porte l'ordre de tri d'un classeur.
///
/// Les trois codes, séparés par des virgules — `numero,artwork,rarete`. Le
/// même vocabulaire que `app_config.json`, pour qu'une valeur se lise dans les
/// deux fichiers sans traduction.
pub const CLE_ORDRE: &str = "ordre_tri_criteres";

/// Lit les réglages propres à un classeur.
///
/// Une base illisible rend des réglages vides : le classeur suivra les
/// Options, ce qui est exactement ce qu'il faut faire d'un classeur qu'on
/// n'arrive pas à ouvrir.
#[must_use]
pub fn reglages_du_classeur(chemin_db: &Path) -> Reglages {
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(chemin_db) else {
        return Reglages::default();
    };
    let valeur = |cle: &str| -> Option<String> {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [cle], |l| l.get(0))
            .ok()
    };
    // Les deux moitiés de la grille vont ensemble : une seule des deux ne dit
    // rien d'exploitable, et l'ancien lecteur complétait déjà avec le défaut.
    let grille = match (
        valeur("colonnes").and_then(|v| v.trim().parse::<u8>().ok()),
        valeur("lignes").and_then(|v| v.trim().parse::<u8>().ok()),
    ) {
        (Some(colonnes), Some(lignes)) => Some(Grille {
            colonnes: colonnes.clamp(GRILLE_MIN, GRILLE_MAX),
            lignes: lignes.clamp(GRILLE_MIN, GRILLE_MAX),
        }),
        _ => None,
    };
    let ordre = valeur(CLE_ORDRE).filter(|v| !v.trim().is_empty()).map(|v| {
        let codes: Vec<&str> = v.split(',').map(str::trim).collect();
        ygo_core::config::normaliser_ordre_tri(&codes)
    });
    Reglages { grille, ordre }
}

/// L'ordre de tri d'un classeur, ou celui des Options s'il n'en a pas.
///
/// Le pendant de [`grille_du_classeur`], et il se lit au même endroit : la
/// table `meta` du classeur.
#[must_use]
pub fn ordre_du_classeur(chemin_db: &Path, defaut: [CritereTri; 3]) -> [CritereTri; 3] {
    reglages_du_classeur(chemin_db).ordre.unwrap_or(defaut)
}

/// Écrit les réglages d'un classeur.
///
/// Un champ à `None` **efface** sa clé : le classeur revient aux Options, et
/// il y revient vraiment — pas en recopiant la valeur du moment, qui figerait
/// le défaut d'aujourd'hui.
///
/// # Errors
///
/// Rend une erreur si le classeur n'est pas ouvrable en écriture.
pub fn definir_reglages(chemin_db: &Path, reglages: &Reglages) -> Result<()> {
    let mut conn = ygo_db::connexion::ouvrir(chemin_db)?;
    definir_reglages_sur(&mut conn, reglages)
}

/// Écrit les réglages **sur une connexion déjà ouverte**.
///
/// L'écran d'un classeur en tient une, en écriture, tant qu'il est ouvert
/// (règle R3 : un écrivain par base). Lui faire ouvrir une seconde connexion
/// pour trois lignes de `meta` serait précisément ce que la règle interdit —
/// d'où cette variante, qui est celle que l'interface appelle.
///
/// # Errors
///
/// Rend une erreur si la transaction échoue.
pub fn definir_reglages_sur(conn: &mut Connection, reglages: &Reglages) -> Result<()> {
    let transaction = conn.transaction()?;
    match reglages.grille {
        Some(g) => {
            for (cle, valeur) in [
                ("colonnes", g.colonnes.clamp(GRILLE_MIN, GRILLE_MAX)),
                ("lignes", g.lignes.clamp(GRILLE_MIN, GRILLE_MAX)),
            ] {
                transaction.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    (cle, valeur.to_string()),
                )?;
            }
        }
        None => {
            transaction.execute("DELETE FROM meta WHERE key IN ('colonnes','lignes')", ())?;
        }
    }
    match reglages.ordre {
        Some(ordre) => {
            let codes: Vec<&str> = ordre.iter().map(|c| c.code()).collect();
            transaction.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (CLE_ORDRE, codes.join(",")),
            )?;
        }
        None => {
            transaction.execute("DELETE FROM meta WHERE key = ?1", [CLE_ORDRE])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Les écarts de création
// ─────────────────────────────────────────────────────────────────────────────

/// La clé de `meta` qui porte les écarts constatés à la création.
///
/// Elle vit dans la base du classeur, et pas dans un fichier à côté, pour une
/// raison pratique : un classeur se déplace, se met à la corbeille et en
/// revient d'un seul fichier. Un journal rangé ailleurs se serait perdu au
/// premier de ces gestes.
pub const CLE_ECARTS: &str = "ecarts_creation";

/// Ce que la création a dû corriger ou refuser, consigné dans le classeur.
///
/// C'est la réponse à une question que l'utilisateur pose légitimement devant
/// un classeur de 62 cartes annoncé à 68 : *où sont passées les six autres ?*
/// Rien n'est retiré en silence — l'écran du classeur en montre une ligne, et
/// le détail est là.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ecarts {
    /// Horodatage de la création, `AAAA-MM-JJ HH:MM:SS` local.
    #[serde(default)]
    pub quand: String,
    /// Lignes dont le libellé a été ramené à la forme des Options.
    #[serde(default)]
    pub canonisees: usize,
    /// Lignes écartées parce que leur « rareté » est un marqueur d'artwork.
    #[serde(default)]
    pub fantomes: Vec<crate::raretes::Fantome>,
    /// Libellés qu'aucune table ne reconnaît, **laissés tels quels**, et leur
    /// nombre de lignes.
    #[serde(default)]
    pub inconnues: BTreeMap<String, usize>,
}

impl Ecarts {
    /// N'y a-t-il rien à signaler ?
    #[must_use]
    pub fn est_vide(&self) -> bool {
        self.canonisees == 0 && self.fantomes.is_empty() && self.inconnues.is_empty()
    }

    /// Le nombre de lignes retirées du classeur.
    #[must_use]
    pub fn retirees(&self) -> usize {
        self.fantomes.len()
    }

    /// Le nombre de lignes gardées avec un libellé non reconnu.
    #[must_use]
    pub fn lignes_inconnues(&self) -> usize {
        self.inconnues.values().sum()
    }

    /// La phrase à montrer dans l'écran du classeur, ou `None` s'il n'y a
    /// rien à dire.
    ///
    /// Elle tient sur une ligne et dit **ce qui a changé**, pas ce qui a été
    /// fait au sens technique : « 6 lignes écartées » se comprend sans savoir
    /// ce qu'est une canonisation.
    #[must_use]
    pub fn resume(&self) -> Option<String> {
        let mut morceaux = Vec::new();
        if self.retirees() > 0 {
            morceaux.push(format!(
                "{} ligne{} écartée{} (fausse rareté)",
                self.retirees(),
                pluriel(self.retirees()),
                pluriel(self.retirees())
            ));
        }
        let inconnues = self.lignes_inconnues();
        if inconnues > 0 {
            morceaux.push(format!(
                "{inconnues} libellé{} non reconnu{}",
                pluriel(inconnues),
                pluriel(inconnues)
            ));
        }
        if self.canonisees > 0 {
            morceaux.push(format!(
                "{} rareté{} harmonisée{}",
                self.canonisees,
                pluriel(self.canonisees),
                pluriel(self.canonisees)
            ));
        }
        (!morceaux.is_empty()).then(|| morceaux.join(" · "))
    }
}

/// `s` s'il en faut un.
fn pluriel(n: usize) -> &'static str {
    if n > 1 {
        "s"
    } else {
        ""
    }
}

/// Consigne les écarts dans la base du classeur.
///
/// Un bilan vide **efface** la clé : un classeur recréé proprement ne doit pas
/// traîner le bandeau de sa version précédente.
///
/// # Errors
///
/// Rend une erreur si la base n'est pas ouvrable en écriture, ou si le JSON
/// ne peut pas être produit.
pub fn enregistrer_ecarts(chemin_db: &Path, ecarts: &Ecarts) -> Result<()> {
    let conn = ygo_db::connexion::ouvrir(chemin_db)?;
    enregistrer_ecarts_sur(&conn, ecarts)
}

/// Consigne les écarts **sur une connexion déjà ouverte**.
///
/// C'est la variante que la création appelle, dans la transaction qui écrit
/// les cartes : le classeur et la note qui explique sa forme entrent
/// ensemble, et une seconde connexion n'a pas à être ouverte sur une base
/// qu'on vient tout juste de refermer.
///
/// # Errors
///
/// Rend une erreur si l'écriture échoue, ou si le JSON ne peut pas être
/// produit.
pub fn enregistrer_ecarts_sur(conn: &Connection, ecarts: &Ecarts) -> Result<()> {
    if ecarts.est_vide() {
        conn.execute("DELETE FROM meta WHERE key = ?1", [CLE_ECARTS])?;
        return Ok(());
    }
    let json = serde_json::to_string(ecarts)?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (CLE_ECARTS, json),
    )?;
    Ok(())
}

/// Lit les écarts consignés à la création, s'il y en a.
///
/// Une base illisible, une clé absente ou un JSON qu'on ne sait plus relire
/// rendent `None` : le bandeau disparaît, le classeur s'ouvre. Un journal
/// d'anomalies n'a pas le droit d'empêcher de consulter sa collection.
#[must_use]
pub fn ecarts_du_classeur(chemin_db: &Path) -> Option<Ecarts> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(chemin_db).ok()?;
    let json: String = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [CLE_ECARTS], |l| {
            l.get(0)
        })
        .ok()?;
    let ecarts: Ecarts = serde_json::from_str(&json).ok()?;
    (!ecarts.est_vide()).then_some(ecarts)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    /// Un classeur jetable avec sa table `meta`.
    fn classeur_vide() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("X.db");
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        (tmp, chemin)
    }

    /// Sans rien en base, le classeur suit les Options.
    #[test]
    fn un_classeur_neuf_suit_les_options() {
        let (_tmp, chemin) = classeur_vide();
        let r = reglages_du_classeur(&chemin);
        assert_eq!(r, Reglages::default());
        assert!(r.suit_les_options());
        assert_eq!(
            grille_du_classeur(&chemin, (4, 5)),
            Grille {
                colonnes: 4,
                lignes: 5
            }
        );
        assert_eq!(
            ordre_du_classeur(
                &chemin,
                [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
            ),
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
        );
    }

    /// L'aller-retour : ce qu'on écrit est ce qu'on relit.
    #[test]
    fn les_reglages_propres_priment_sur_les_options() {
        let (_tmp, chemin) = classeur_vide();
        let voulu = Reglages {
            grille: Some(Grille {
                colonnes: 5,
                lignes: 4,
            }),
            ordre: Some([CritereTri::Artwork, CritereTri::Rarete, CritereTri::Numero]),
        };
        definir_reglages(&chemin, &voulu).unwrap();

        assert_eq!(reglages_du_classeur(&chemin), voulu);
        assert!(!reglages_du_classeur(&chemin).suit_les_options());
        assert_eq!(
            grille_du_classeur(&chemin, (3, 3)),
            Grille {
                colonnes: 5,
                lignes: 4
            },
            "le classeur l'emporte sur le défaut"
        );
        assert_eq!(
            ordre_du_classeur(&chemin, ygo_core::config::ORDRE_TRI_DEFAUT),
            [CritereTri::Artwork, CritereTri::Rarete, CritereTri::Numero]
        );

        // Réécrire remplace, ne double pas : `meta.key` est une clé primaire.
        definir_reglages(&chemin, &voulu).unwrap();
        assert_eq!(reglages_du_classeur(&chemin), voulu);
    }

    /// Revenir aux Options **efface** la clé — il ne fige pas la valeur du
    /// moment.
    ///
    /// La nuance compte : recopier le défaut d'aujourd'hui rendrait le
    /// classeur sourd à un changement d'Options demain, sans que rien ne le
    /// dise.
    #[test]
    fn revenir_aux_options_efface_au_lieu_de_figer() {
        let (_tmp, chemin) = classeur_vide();
        definir_reglages(
            &chemin,
            &Reglages {
                grille: Some(Grille {
                    colonnes: 5,
                    lignes: 4,
                }),
                ordre: Some([CritereTri::Artwork, CritereTri::Rarete, CritereTri::Numero]),
            },
        )
        .unwrap();
        definir_reglages(&chemin, &Reglages::default()).unwrap();

        assert_eq!(reglages_du_classeur(&chemin), Reglages::default());
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        let restantes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key IN ('colonnes','lignes',?1)",
                [CLE_ORDRE],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(restantes, 0, "les clés sont parties");
        // Et le classeur suit de nouveau ce que les Options diront.
        assert_eq!(
            grille_du_classeur(&chemin, (7, 7)),
            Grille {
                colonnes: 7,
                lignes: 7
            }
        );
    }

    /// Une grille hors bornes est ramenée, jamais refusée.
    #[test]
    fn une_grille_hors_bornes_est_ramenee() {
        let (_tmp, chemin) = classeur_vide();
        definir_reglages(
            &chemin,
            &Reglages {
                grille: Some(Grille {
                    colonnes: 99,
                    lignes: 1,
                }),
                ordre: None,
            },
        )
        .unwrap();
        assert_eq!(
            reglages_du_classeur(&chemin).grille,
            Some(Grille {
                colonnes: GRILLE_MAX,
                lignes: GRILLE_MIN
            })
        );
    }

    /// Une valeur abîmée à la main ne fait pas paniquer la lecture.
    #[test]
    fn une_valeur_illisible_retombe_sur_les_options() {
        let (_tmp, chemin) = classeur_vide();
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('colonnes','quatre'),('lignes','5'),(?1,'zzz')",
            [CLE_ORDRE],
        )
        .unwrap();
        let r = reglages_du_classeur(&chemin);
        assert_eq!(
            r.grille, None,
            "une moitié illisible : pas de grille propre"
        );
        // « zzz » n'est pas un critère : la normalisation complète avec
        // l'ordre **canonique** — qui n'est pas l'ordre par défaut des
        // Options (`numero, artwork, rarete`). La clé existe, elle est juste
        // sans contenu exploitable.
        assert_eq!(
            r.ordre,
            Some([CritereTri::Numero, CritereTri::Rarete, CritereTri::Artwork])
        );
    }

    fn carte(rowid: i64, nom: &str, code: &str, rarete: &str, quantite: i64) -> Carte {
        Carte {
            rowid,
            nom: nom.to_owned(),
            rarete: rarete.to_owned(),
            set_code: code.to_owned(),
            quantite,
            possedee: quantite > 0,
            ..Carte::default()
        }
    }

    fn cartes(n: usize) -> Vec<Carte> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_possible_wrap)]
                carte(i as i64 + 1, "Carte", &format!("S-EN{i:03}"), "Common", 0)
            })
            .collect()
    }

    fn ids(tranche: &[Carte]) -> Vec<i64> {
        tranche.iter().map(|c| c.rowid).collect()
    }

    // ── La pagination, avec des grilles que les données réelles ignorent ────
    //
    // Les 26 classeurs sont tous en 3×3. Ces tests emploient exprès les autres
    // configurations que l'utilisateur peut choisir.

    #[test]
    fn le_nombre_de_doubles_pages_suit_la_grille_choisie() {
        let n = 100;
        // 3×3 → 9 par page → 12 pages → 1 + ⌈11/2⌉ = 7
        assert_eq!(
            nb_doubles_pages(
                n,
                Grille {
                    colonnes: 3,
                    lignes: 3
                }
            ),
            7
        );
        // 4×3 → 12 par page → 9 pages → 1 + ⌈8/2⌉ = 5
        assert_eq!(
            nb_doubles_pages(
                n,
                Grille {
                    colonnes: 4,
                    lignes: 3
                }
            ),
            5
        );
        // 4×4 → 16 par page → 7 pages → 1 + ⌈6/2⌉ = 4
        assert_eq!(
            nb_doubles_pages(
                n,
                Grille {
                    colonnes: 4,
                    lignes: 4
                }
            ),
            4
        );
        // 2×2 → 4 par page → 25 pages → 1 + ⌈24/2⌉ = 13
        assert_eq!(
            nb_doubles_pages(
                n,
                Grille {
                    colonnes: 2,
                    lignes: 2
                }
            ),
            13
        );
    }

    #[test]
    fn un_classeur_vide_a_quand_meme_une_double_page() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        assert_eq!(nb_doubles_pages(0, grille), 1);
        let (g, d) = double_page(&[], 0, grille);
        assert!(g.is_empty() && d.is_empty());
    }

    #[test]
    fn une_page_exactement_pleine_ne_deborde_pas_sur_une_seconde() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        assert_eq!(nb_doubles_pages(9, grille), 1, "neuf cartes, une page");
        assert_eq!(nb_doubles_pages(10, grille), 2, "la dixième ouvre la suite");
    }

    #[test]
    fn la_premiere_double_page_est_seule_a_droite() {
        let grille = Grille {
            colonnes: 2,
            lignes: 2,
        };
        let toutes = cartes(20);
        let (gauche, droite) = double_page(&toutes, 0, grille);
        assert!(gauche.is_empty(), "rien à gauche du premier feuillet");
        assert_eq!(ids(droite), [1, 2, 3, 4]);
    }

    #[test]
    fn l_invariant_de_premiere_page_vaut_pour_toutes_les_grilles() {
        // Règle immuable : quelle que soit la grille et quel que soit le
        // nombre de cartes, le premier feuillet ne montre qu'une page.
        for (colonnes, lignes) in [(3, 3), (4, 3), (4, 4), (2, 2), (1, 1), (5, 4)] {
            let grille = Grille { colonnes, lignes };
            for n in [0, 1, 5, 9, 12, 16, 100, 1120] {
                let toutes = cartes(n);
                let (gauche, droite) = double_page(&toutes, 0, grille);
                assert!(
                    gauche.is_empty(),
                    "grille {colonnes}×{lignes}, {n} cartes : la première page \
                     doit être seule à droite"
                );
                assert_eq!(
                    droite.len(),
                    n.min(grille.par_page()),
                    "grille {colonnes}×{lignes}, {n} cartes : la page de droite \
                     porte les premières cartes"
                );
            }
        }
    }

    #[test]
    fn les_doubles_pages_suivantes_vont_par_deux() {
        let grille = Grille {
            colonnes: 2,
            lignes: 2,
        };
        let toutes = cartes(20);
        // Double page 1 : pages 1 et 2 → cartes 5..8 et 9..12
        let (g, d) = double_page(&toutes, 1, grille);
        assert_eq!(ids(g), [5, 6, 7, 8]);
        assert_eq!(ids(d), [9, 10, 11, 12]);
        // Double page 2 : pages 3 et 4 → cartes 13..16 et 17..20
        let (g, d) = double_page(&toutes, 2, grille);
        assert_eq!(ids(g), [13, 14, 15, 16]);
        assert_eq!(ids(d), [17, 18, 19, 20]);
    }

    #[test]
    fn une_derniere_page_incomplete_rend_ce_qui_reste() {
        let grille = Grille {
            colonnes: 2,
            lignes: 2,
        };
        let toutes = cartes(6);
        let (g, d) = double_page(&toutes, 1, grille);
        assert_eq!(ids(g), [5, 6], "deux cartes seulement");
        assert!(d.is_empty(), "et rien en face");
    }

    #[test]
    fn une_double_page_hors_limites_ne_leve_pas() {
        // Le découpage Python ne lève jamais : il rend des tranches vides.
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let toutes = cartes(5);
        let (g, d) = double_page(&toutes, 99, grille);
        assert!(g.is_empty() && d.is_empty());
    }

    #[test]
    fn une_grille_degeneree_ne_divise_pas_par_zero() {
        let grille = Grille {
            colonnes: 0,
            lignes: 3,
        };
        assert_eq!(nb_doubles_pages(50, grille), 1);
        let toutes = cartes(50);
        let (g, d) = double_page(&toutes, 0, grille);
        assert!(g.is_empty() && d.is_empty());
    }

    #[test]
    fn les_numeros_de_page_affiches_suivent_le_feuilletage() {
        assert_eq!(numeros_de_page(0), (None, 1), "la page 1 est seule");
        assert_eq!(numeros_de_page(1), (Some(2), 3));
        assert_eq!(numeros_de_page(2), (Some(4), 5));
    }

    // ── Les filtres ─────────────────────────────────────────────────────────

    fn jeu() -> Vec<Carte> {
        vec![
            carte(1, "Dark Magician", "RA05-EN001", "Ultra Rare", 2),
            carte(2, "Blue-Eyes White Dragon", "RA05-EN002", "Secret Rare", 0),
            carte(3, "Dark Magician Girl", "RA05-EN003", "Ultra Rare", 1),
        ]
    }

    #[test]
    fn la_recherche_porte_sur_le_nom_et_aussi_sur_le_code() {
        let p = Priorites::default();
        let par_nom = Filtres {
            terme: "magician".to_owned(),
            ..Filtres::default()
        };
        assert_eq!(
            appliquer(jeu(), &par_nom, &p)
                .iter()
                .map(|c| c.rowid)
                .collect::<Vec<_>>(),
            [1, 3]
        );

        let par_code = Filtres {
            terme: "en002".to_owned(),
            ..Filtres::default()
        };
        assert_eq!(
            appliquer(jeu(), &par_code, &p)
                .iter()
                .map(|c| c.rowid)
                .collect::<Vec<_>>(),
            [2],
            "le code de set aussi, sans casse"
        );
    }

    #[test]
    fn un_numero_seul_suffit_a_retrouver_la_carte() {
        let p = Priorites::default();
        let cherche = |t: &str| {
            appliquer(
                jeu(),
                &Filtres {
                    terme: t.to_owned(),
                    ..Filtres::default()
                },
                &p,
            )
            .iter()
            .map(|c| c.rowid)
            .collect::<Vec<_>>()
        };
        assert_eq!(cherche("002"), [2], "le numéro nu, sans le préfixe de set");
        assert_eq!(cherche("2"), [2], "et même sans les zéros de tête");
        assert_eq!(cherche("ra05en003"), [3], "séparateur omis");
        assert_eq!(cherche("RA05-EN003"), [3], "code complet");
        assert_eq!(cherche("  002  "), [2], "espaces alentour ignorés");
    }

    /// Le cas qui a fait croire à une recherche cassée : `080` ne ramène rien
    /// sur `RA02`, non pas parce que la recherche exige le code entier, mais
    /// parce que ce set s'arrête à `079`. Le filtre a raison de ne rien
    /// ramener ; c'est à l'écran d'expliquer pourquoi.
    #[test]
    fn un_numero_absent_du_classeur_ne_ramene_rien() {
        let p = Priorites::default();
        let vide = appliquer(
            jeu(),
            &Filtres {
                terme: "080".to_owned(),
                ..Filtres::default()
            },
            &p,
        );
        assert!(vide.is_empty());
    }

    /// La passe compacte est une **disjonction** : elle ajoute des
    /// correspondances, elle n'en retire aucune. Le terme `-EN0`, figé par les
    /// oracles, dépend du tiret et doit continuer de tout ramener.
    #[test]
    fn la_passe_compacte_n_enleve_jamais_de_resultat() {
        let p = Priorites::default();
        let avec_tiret = appliquer(
            jeu(),
            &Filtres {
                terme: "-EN0".to_owned(),
                ..Filtres::default()
            },
            &p,
        );
        assert_eq!(avec_tiret.len(), 3);
    }

    #[test]
    fn la_possession_se_lit_sur_la_quantite_et_non_sur_le_drapeau() {
        // ÉCART QUE LES DONNÉES RÉELLES NE RÉVÈLENT PAS : sur les 7 136 cartes
        // figées, `possessed` et `quantite > 0` s'accordent toujours. Le Python
        // filtre pourtant sur la QUANTITÉ, et les deux peuvent diverger — une
        // carte marquée possédée dont la quantité est retombée à zéro.
        let p = Priorites::default();
        let mut cartes = jeu();
        cartes[1].possedee = true; // drapeau vrai…
        cartes[1].quantite = 0; // …mais quantité nulle

        let possedees = Filtres {
            possession: Possession::Possedees,
            ..Filtres::default()
        };
        assert_eq!(
            appliquer(cartes.clone(), &possedees, &p)
                .iter()
                .map(|c| c.rowid)
                .collect::<Vec<_>>(),
            [1, 3],
            "la carte au drapeau menteur ne doit PAS passer"
        );

        let non_possedees = Filtres {
            possession: Possession::NonPossedees,
            ..Filtres::default()
        };
        assert_eq!(
            appliquer(cartes, &non_possedees, &p)
                .iter()
                .map(|c| c.rowid)
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn le_filtre_de_rarete_est_une_egalite_exacte() {
        let p = Priorites::default();
        let f = Filtres {
            rarete: Some("Ultra Rare".to_owned()),
            ..Filtres::default()
        };
        assert_eq!(appliquer(jeu(), &f, &p).len(), 2);

        let inexistante = Filtres {
            rarete: Some("ultra rare".to_owned()),
            ..Filtres::default()
        };
        assert!(
            appliquer(jeu(), &inexistante, &p).is_empty(),
            "la liste déroulante propose les libellés exacts : pas de tolérance"
        );
    }

    #[test]
    fn le_filtre_n_raretes_vient_en_dernier() {
        // La raison d'être de cet ordre, en un cas : l'utilisateur filtre sur
        // « Possédées » et ne garde qu'une rareté par artwork. Il veut la plus
        // rare DE SES cartes — pas la plus rare dans l'absolu, qu'il pourrait
        // ne pas posséder et qui lui laisserait une page vide.
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("rarity_config.json");
        std::fs::write(
            &chemin,
            r#"{"Secret Rare": 10, "Ultra Rare": 5, "Common": 1}"#,
        )
        .unwrap();
        let p = Priorites::charger(&chemin);

        // Même numéro, même artwork : un seul groupe de trois raretés.
        // La plus rare — Secret Rare — n'est PAS possédée.
        let cartes = vec![
            Carte {
                card_image_id: 42,
                ..carte(1, "Dark Magician", "RA05-EN001", "Secret Rare", 0)
            },
            Carte {
                card_image_id: 42,
                ..carte(2, "Dark Magician", "RA05-EN001", "Ultra Rare", 3)
            },
            Carte {
                card_image_id: 42,
                ..carte(3, "Dark Magician", "RA05-EN001", "Common", 1)
            },
        ];

        let filtres = Filtres {
            possession: Possession::Possedees,
            n_raretes: 1,
            ..Filtres::default()
        };
        let retenues = appliquer(cartes, &filtres, &p);
        assert_eq!(
            retenues.iter().map(|c| c.rowid).collect::<Vec<_>>(),
            [2],
            "attendu la plus rare DES POSSÉDÉES. Le filtre N-raretés appliqué \
             en premier aurait élu la Secret Rare, que la possession aurait \
             ensuite supprimée — page vide. Ne pas l'appliquer du tout aurait \
             laissé la Common en plus."
        );
    }

    #[test]
    fn n_raretes_a_zero_ne_filtre_rien() {
        let p = Priorites::default();
        let f = Filtres {
            n_raretes: 0,
            ..Filtres::default()
        };
        assert_eq!(
            appliquer(jeu(), &f, &p).len(),
            3,
            "zéro force « toutes les raretés », y compris contre le réglage global"
        );
    }

    #[test]
    fn les_filtres_se_combinent() {
        let p = Priorites::default();
        let f = Filtres {
            terme: "magician".to_owned(),
            rarete: Some("Ultra Rare".to_owned()),
            possession: Possession::Possedees,
            n_raretes: 0,
        };
        assert_eq!(appliquer(jeu(), &f, &p).len(), 2);
    }

    #[test]
    fn les_raretes_proposees_sont_triees_et_sans_doublon() {
        assert_eq!(
            raretes_disponibles(&jeu()),
            ["Secret Rare", "Ultra Rare"],
            "deux Ultra Rare ne donnent qu'une entrée"
        );
        let sans = vec![carte(1, "X", "S-EN001", "", 0)];
        assert!(
            raretes_disponibles(&sans).is_empty(),
            "une rareté vide n'est pas proposée"
        );
    }

    // ── Le nom de fichier d'image ───────────────────────────────────────────

    #[test]
    fn le_fichier_est_le_dernier_segment_du_chemin() {
        assert_eq!(
            fichier_depuis_url("https://images.ygoprodeck.com/images/cards/12345.jpg").as_deref(),
            Some("12345.jpg")
        );
        assert_eq!(
            fichier_depuis_url("https://ms.yugipedia.com//9/96/DarkMagician-RA05-EN-UR.png")
                .as_deref(),
            Some("DarkMagician-RA05-EN-UR.png"),
            "la double barre de Yugipedia ne gêne pas"
        );
    }

    #[test]
    fn la_requete_et_le_fragment_ne_font_pas_partie_du_fichier() {
        assert_eq!(
            fichier_depuis_url("https://exemple/img/carte.jpg?v=2").as_deref(),
            Some("carte.jpg")
        );
        assert_eq!(
            fichier_depuis_url("https://exemple/img/carte.jpg#haut").as_deref(),
            Some("carte.jpg")
        );
    }

    #[test]
    fn une_url_sans_fichier_ne_rend_rien() {
        assert_eq!(fichier_depuis_url(""), None);
        assert_eq!(fichier_depuis_url("https://exemple.com/"), None);
        assert_eq!(fichier_depuis_url("https://exemple.com"), None);
    }

    // ── La taille adaptative ────────────────────────────────────────────────

    const BORNES: BornesCarte = BornesCarte { min: 90, max: 813 };

    /// Une fenêtre, en clair.
    const fn fenetre(largeur: u32, hauteur: u32) -> Fenetre {
        Fenetre { largeur, hauteur }
    }

    /// Une grille carrée de `n` de côté.
    const fn carree(n: u8) -> Grille {
        Grille {
            colonnes: n,
            lignes: n,
        }
    }

    #[test]
    fn la_carte_garde_le_ratio_d_une_carte_yugioh() {
        let (l, h) = taille_carte(fenetre(1920, 1440), carree(3), BORNES);
        // 59:86 — on tolère l'arrondi entier.
        let attendu = l * RATIO_HAUTEUR / RATIO_LARGEUR;
        assert_eq!(h, attendu);
        assert!(h > l, "une carte est plus haute que large");
    }

    #[test]
    fn plus_de_colonnes_donne_des_cartes_plus_petites() {
        // Fenêtre volontairement très haute : on isole l'effet des colonnes,
        // sinon c'est la hauteur qui décide et le nombre de colonnes ne
        // change plus rien.
        let large = taille_carte(fenetre(1600, 4000), carree(3), BORNES).0;
        let etroit = taille_carte(
            fenetre(1600, 4000),
            Grille {
                colonnes: 6,
                lignes: 3,
            },
            BORNES,
        )
        .0;
        assert!(
            etroit < large,
            "{etroit} devrait être plus petit que {large}"
        );
    }

    #[test]
    fn la_largeur_de_carte_reste_dans_ses_bornes() {
        // Fenêtre minuscule : on ne descend pas sous le minimum.
        assert_eq!(
            taille_carte(fenetre(200, 200), carree(3), BORNES).0,
            BORNES.min
        );
        // Fenêtre gigantesque sur les deux axes : on ne dépasse pas le
        // maximum. Il faut bien les deux — une seule suffisait avant, elle
        // ne suffit plus.
        assert_eq!(
            taille_carte(fenetre(10_000, 10_000), carree(3), BORNES).0,
            BORNES.max
        );
    }

    #[test]
    fn une_fenetre_pas_encore_affichee_retombe_sur_mille_deux_cent_quatre_vingts() {
        // `winfo_width()` rend 1 tant que la fenêtre n'est pas mappée.
        let attendu = taille_carte(fenetre(1280, 800), carree(3), BORNES);
        assert_eq!(taille_carte(fenetre(1, 1), carree(3), BORNES), attendu);
        assert_eq!(taille_carte(fenetre(0, 0), carree(3), BORNES), attendu);
        // Et le repli vaut par axe : une hauteur absurde ne condamne pas une
        // largeur parfaitement valable.
        assert_eq!(
            taille_carte(fenetre(1280, 0), carree(3), BORNES),
            attendu,
            "hauteur seule absurde"
        );
        assert_eq!(
            taille_carte(fenetre(0, 800), carree(3), BORNES),
            attendu,
            "largeur seule absurde"
        );
    }

    #[test]
    fn une_grille_sans_colonne_ne_divise_pas_par_zero() {
        let (l, _) = taille_carte(
            fenetre(1280, 800),
            Grille {
                colonnes: 0,
                lignes: 3,
            },
            BORNES,
        );
        assert!(l >= BORNES.min, "traitée comme une colonne");
        let (h, _) = taille_carte(
            fenetre(1280, 800),
            Grille {
                colonnes: 3,
                lignes: 0,
            },
            BORNES,
        );
        assert!(h >= BORNES.min, "et une grille sans ligne non plus");
    }

    #[test]
    fn la_grille_par_page_multiplie_colonnes_et_lignes() {
        assert_eq!(
            Grille {
                colonnes: 3,
                lignes: 3
            }
            .par_page(),
            9
        );
        assert_eq!(
            Grille {
                colonnes: 4,
                lignes: 3
            }
            .par_page(),
            12
        );
        assert_eq!(
            Grille {
                colonnes: 4,
                lignes: 4
            }
            .par_page(),
            16
        );
    }

    /// Aller à une page mène au feuillet qui la montre — vérifié contre
    /// [`numeros_de_page`] et non contre une formule réécrite.
    ///
    /// # Ce qui rend ce test discriminant
    ///
    /// Il ne recalcule rien : pour chaque feuillet, il demande à
    /// `numeros_de_page` quelles pages il montre, puis exige que chacune de
    /// ces pages ramène **à ce feuillet-là**. Une inverse fausse d'un cran —
    /// l'erreur naturelle, puisque le premier feuillet ne montre qu'une page —
    /// tombe immédiatement.
    #[test]
    fn aller_a_une_page_mene_au_feuillet_qui_la_montre() {
        let feuillets = 19;
        for indice in 0..feuillets {
            let (gauche, droite) = numeros_de_page(indice);
            for page in gauche.into_iter().chain(std::iter::once(droite)) {
                assert_eq!(
                    feuillet_de_page(page, feuillets),
                    indice,
                    "la page {page} est montrée par le feuillet {indice}"
                );
            }
        }
    }

    /// Les bords : page absente, page au-delà, classeur d'un seul feuillet.
    #[test]
    fn le_saut_de_page_reste_dans_le_classeur() {
        assert_eq!(feuillet_de_page(0, 19), 0, "il n'y a pas de page 0");
        assert_eq!(feuillet_de_page(1, 19), 0);
        assert_eq!(feuillet_de_page(37, 19), 18, "la dernière page du classeur");
        assert_eq!(
            feuillet_de_page(38, 19),
            18,
            "au-delà : on s'arrête à la fin"
        );
        assert_eq!(feuillet_de_page(usize::MAX, 19), 18, "et sans déborder");
        // Un classeur d'un seul feuillet n'a qu'une destination.
        for page in [0, 1, 2, 99] {
            assert_eq!(feuillet_de_page(page, 1), 0, "page {page}");
        }
        // Aucun feuillet : on ne panique pas, et on ne rend pas d'indice.
        assert_eq!(feuillet_de_page(5, 0), 0);
    }

    /// La dernière page annoncée est bien la plus grande que le classeur porte.
    ///
    /// Confrontée à `numeros_de_page`, là encore : si elle mentait, le champ
    /// de saisie interdirait une page qui existe, ou en proposerait une qui
    /// n'existe pas.
    #[test]
    fn la_derniere_page_est_celle_du_dernier_feuillet() {
        for feuillets in 1..40_usize {
            let (_, droite) = numeros_de_page(feuillets - 1);
            assert_eq!(derniere_page(feuillets), droite, "{feuillets} feuillet(s)");
            // Et elle mène bien au dernier feuillet.
            assert_eq!(
                feuillet_de_page(derniere_page(feuillets), feuillets),
                feuillets - 1
            );
        }
    }

    // ── Centrage et navigation par la recherche ─────────────────────────────

    /// Sur un écran 16:9, la hauteur limite avant la largeur, et le classeur
    /// n'occupe plus toute la largeur : c'est ce reste qu'il faut répartir des
    /// deux côtés. Sur une fenêtre étroite, il n'y a rien à répartir.
    ///
    /// # Ce que ce test disait avant
    ///
    /// La même chose, mais pour la mauvaise raison : le reste venait de la
    /// borne `CARD_W_MAX = 220`, atteinte dès 1 700 points de fenêtre. La
    /// borne est maintenant à la définition des images et n'est plus jamais
    /// atteinte ; le reste, lui, demeure — une double page 3×3 a un rapport
    /// de 1,37 quand un écran 16:9 en a 1,78. Il est géométrique, pas
    /// arbitraire, et le centrage reste nécessaire.
    #[test]
    fn la_largeur_occupee_laisse_du_reste_sur_un_ecran_seize_neuvieme() {
        let grille = carree(3);

        let (etroite, _) = taille_carte(fenetre(1280, 1024), grille, BORNES);
        let reste_etroit = 1280_u32.saturating_sub(largeur_double_page(etroite, grille));

        let (large, _) = taille_carte(fenetre(2560, 1440), grille, BORNES);
        assert!(large < BORNES.max, "la borne n'est pas ce qui limite");
        let reste_large = 2560_u32.saturating_sub(largeur_double_page(large, grille));

        // Le reste n'est plus le gouffre d'avant — l'ajustement en hauteur en
        // reprend la plus grande part — mais il reste de l'ordre du quart de
        // l'écran : assez pour que l'ignorer décale visiblement le classeur.
        assert!(
            reste_large > reste_etroit,
            "le reste s'ouvre sur écran large : {reste_etroit} puis {reste_large}"
        );
        assert!(
            reste_large > 200,
            "et il vaut la peine d'être réparti : {reste_large}"
        );
    }

    /// Sur un écran large, c'est la **hauteur** qui décide.
    ///
    /// Le test qui compte pour ce lot. Il ne réécrit pas la formule : il
    /// regarde à quoi la taille réagit. Agrandir la fenêtre en hauteur donne
    /// des cartes plus grandes ; l'agrandir en largeur ne change plus rien,
    /// parce que la largeur n'est plus ce qui manque.
    #[test]
    fn sur_un_ecran_large_c_est_la_hauteur_qui_decide() {
        let grille = carree(3);
        let (reference, _) = taille_carte(fenetre(2560, 1440), grille, BORNES);

        let (plus_haut, _) = taille_carte(fenetre(2560, 1800), grille, BORNES);
        assert!(
            plus_haut > reference,
            "300 points de hauteur en plus doivent agrandir la carte : \
             {reference} puis {plus_haut}"
        );

        let (plus_large, _) = taille_carte(fenetre(3200, 1440), grille, BORNES);
        assert_eq!(
            plus_large, reference,
            "640 points de largeur en plus ne changent rien : ce n'est pas \
             la largeur qui manque"
        );
    }

    /// Sur une fenêtre haute et étroite, c'est l'inverse.
    ///
    /// La contrepartie du test précédent. Sans elle, une implémentation qui
    /// ne regarderait *que* la hauteur passerait l'autre sans broncher.
    #[test]
    fn sur_une_fenetre_haute_et_etroite_c_est_la_largeur_qui_decide() {
        let grille = carree(3);
        let (reference, _) = taille_carte(fenetre(1200, 2000), grille, BORNES);

        let (plus_large, _) = taille_carte(fenetre(1500, 2000), grille, BORNES);
        assert!(
            plus_large > reference,
            "300 points de largeur en plus doivent agrandir la carte : \
             {reference} puis {plus_large}"
        );

        let (plus_haut, _) = taille_carte(fenetre(1200, 2400), grille, BORNES);
        assert_eq!(
            plus_haut, reference,
            "400 points de hauteur en plus ne changent rien"
        );
    }

    /// La double page tient dans la fenêtre, **sur les deux axes**.
    ///
    /// L'invariant que tout le calcul sert à garantir. Il ne tenait que sur
    /// un axe : à 2 560 × 1 440 en 3×3, une taille calculée sur la seule
    /// largeur réclamait 1 692 points de haut pour trois lignes, et la double
    /// page passait sous la barre de défilement.
    #[test]
    fn la_double_page_tient_dans_la_fenetre_sur_les_deux_axes() {
        for grille in [
            carree(3),
            Grille {
                colonnes: 4,
                lignes: 3,
            },
            Grille {
                colonnes: 2,
                lignes: 2,
            },
            Grille {
                colonnes: 3,
                lignes: 4,
            },
        ] {
            for (l, h) in [
                (1280, 800),
                (1440, 900),
                (1920, 1080),
                (2560, 1440),
                (3440, 1440),
                (3840, 2160),
            ] {
                let (largeur_carte, hauteur_carte) = taille_carte(fenetre(l, h), grille, BORNES);
                if largeur_carte == BORNES.min {
                    continue; // fenêtre trop petite : la borne basse l'emporte
                }
                assert!(
                    largeur_double_page(largeur_carte, grille) <= l,
                    "{grille:?} en {l}×{h} : trop large"
                );
                assert!(
                    hauteur_double_page(hauteur_carte, grille) <= h,
                    "{grille:?} en {l}×{h} : trop haut"
                );
            }
        }
    }

    /// La largeur occupée **remplit** la fenêtre tant que la borne n'est pas
    /// atteinte : c'est ce que `taille_carte` calcule, à l'envers.
    ///
    /// Sans cette borne inférieure, une mesure qui n'aurait compté qu'une seule
    /// page au lieu de deux passait tous les autres tests — et le classeur se
    /// serait retrouvé centré autour du mauvais milieu.
    #[test]
    fn la_largeur_occupee_remplit_la_fenetre_avant_la_borne() {
        let bornes = BORNES;
        for colonnes in 2..=5_u8 {
            let grille = Grille {
                colonnes,
                lignes: 3,
            };
            // Fenêtre très haute : on isole la contrainte de largeur, seule
            // dont parle cette propriété.
            for largeur in [1000, 1280, 1440] {
                let (l, _) = taille_carte(fenetre(largeur, 4000), grille, bornes);
                if l >= bornes.max || l <= bornes.min {
                    continue; // bornée : il reste forcément de la place
                }
                let utile = largeur - 2 * MARGE_EXTERIEURE - RESERVE_DEFILEMENT;
                let occupe = largeur_double_page(l, grille);
                // La division entière de `taille_carte` perd au plus un point
                // par colonne, deux pages comprises.
                let perte = 2 * u32::from(colonnes);
                assert!(
                    occupe + perte >= utile,
                    "{colonnes} colonnes, fenêtre {largeur} : {occupe} occupés pour {utile} utiles"
                );
                assert!(occupe <= utile, "et sans déborder");
            }
        }
    }

    /// La largeur occupée ne dépasse jamais la fenêtre tant que la borne n'est
    /// pas atteinte — sans quoi centrer pousserait le classeur hors de l'écran.
    #[test]
    fn la_largeur_occupee_tient_dans_la_fenetre() {
        let bornes = BORNES;
        for colonnes in 2..=5_u8 {
            let grille = Grille {
                colonnes,
                lignes: 3,
            };
            for (largeur, hauteur) in [
                (800, 600),
                (1280, 800),
                (1600, 900),
                (1920, 1080),
                (2560, 1440),
                (3840, 2160),
            ] {
                let (l, _) = taille_carte(fenetre(largeur, hauteur), grille, bornes);
                let occupe = largeur_double_page(l, grille);
                if l > bornes.min {
                    assert!(
                        occupe <= largeur,
                        "{colonnes} colonnes, fenêtre {largeur} : {occupe} occupés"
                    );
                }
            }
        }
    }

    /// La conversion rang → feuillet suit l'invariant : page 0 seule, puis par
    /// deux. Elle doit rendre exactement ce que [`double_page`] montre.
    #[test]
    fn le_feuillet_d_une_carte_est_celui_ou_double_page_la_montre() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let cartes: Vec<Carte> = (0..60)
            .map(|i| carte(i64::from(i) + 1, "Nom", "SET-EN001", "Common", 0))
            .collect();

        for position in 0..cartes.len() {
            let feuillet = feuillet_de(position, grille);
            let (gauche, droite) = double_page(&cartes, feuillet, grille);
            let rowid = cartes[position].rowid;
            assert!(
                gauche.iter().chain(droite.iter()).any(|c| c.rowid == rowid),
                "la carte de rang {position} n'est pas sur le feuillet {feuillet}"
            );
        }
    }

    #[test]
    fn les_neuf_premieres_cartes_sont_sur_le_feuillet_zero() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        for position in 0..9 {
            assert_eq!(feuillet_de(position, grille), 0);
        }
        // Le feuillet 1 porte DEUX pages, la 1 et la 2 : il couvre donc les
        // rangs 9 à 26. C'est ce que j'avais d'abord mal compté — le test
        // croisé contre `double_page`, lui, ne s'y est pas trompé.
        assert_eq!(feuillet_de(9, grille), 1, "page 1, à gauche du feuillet 1");
        assert_eq!(feuillet_de(17, grille), 1, "toujours page 1");
        assert_eq!(feuillet_de(18, grille), 1, "page 2, à droite du feuillet 1");
        assert_eq!(feuillet_de(26, grille), 1);
        assert_eq!(feuillet_de(27, grille), 2, "page 3 ouvre le feuillet 2");
    }

    #[test]
    fn feuillet_de_ne_divise_pas_par_zero() {
        let grille = Grille {
            colonnes: 0,
            lignes: 0,
        };
        assert_eq!(feuillet_de(42, grille), 0);
    }

    fn jeu_recherche() -> Vec<Carte> {
        vec![
            carte(1, "Dark Magician", "RA02-EN001", "Ultra Rare", 0),
            carte(2, "Dark Magician", "RA02-EN001", "Secret Rare", 0),
            carte(
                3,
                "Buster Blader, the Dark Destroyer",
                "RA02-EN008",
                "Common",
                0,
            ),
            carte(4, "Rescue Cat", "RA02-EN080", "Common", 0),
        ]
    }

    /// Le numéro exact passe devant, quelle que soit la position dans le
    /// classeur.
    #[test]
    fn le_numero_de_collection_se_lit_a_la_fin_du_code() {
        assert_eq!(numero_de_collection("RA02-EN008"), Some(8));
        assert_eq!(numero_de_collection("LDK2-ENJ01"), Some(1));
        assert_eq!(numero_de_collection("LOCR-JP001"), Some(1));
        assert_eq!(numero_de_collection("SET-ENSP1"), Some(1));
        assert_eq!(numero_de_collection("SANS-CHIFFRE"), None);
        assert_eq!(numero_de_collection(""), None);
    }

    /// Le cas qui a révélé que le rang 0 ne servait à rien : on tape `8`, et
    /// il faut que `SET-EN008` passe devant toutes les cartes dont le nom
    /// contient un 8 — même celles qui la précèdent dans le classeur.
    #[test]
    fn un_numero_nu_passe_devant_les_noms_qui_le_contiennent() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let cartes = vec![
            carte(1, "Level 8 Dragon", "SET-EN001", "Common", 0),
            carte(2, "Number 88", "SET-EN002", "Common", 0),
            carte(3, "Rescue Cat", "SET-EN008", "Common", 0),
        ];
        let s = suggestions(&cartes, "8", grille, MAX_SUGGESTIONS);
        assert_eq!(
            s.first().map(|s| s.rowid),
            Some(3),
            "SET-EN008 d'abord, malgré sa position"
        );
        assert_eq!(s.len(), 3, "les autres suivent, elles correspondent aussi");

        // Avec les zéros de tête, même résultat.
        let s = suggestions(&cartes, "008", grille, MAX_SUGGESTIONS);
        assert_eq!(s.first().map(|s| s.rowid), Some(3));
    }

    #[test]
    fn le_numero_exact_arrive_en_tete() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let s = suggestions(&jeu_recherche(), "RA02-EN008", grille, MAX_SUGGESTIONS);
        assert_eq!(s.first().map(|s| s.rowid), Some(3));

        // Sans le préfixe, ni les séparateurs.
        let s = suggestions(&jeu_recherche(), "ra02en008", grille, MAX_SUGGESTIONS);
        assert_eq!(s.first().map(|s| s.rowid), Some(3));
    }

    /// Un nom qui commence par le terme passe devant un nom qui le contient
    /// au milieu.
    #[test]
    fn un_nom_qui_commence_par_le_terme_passe_devant() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let s = suggestions(&jeu_recherche(), "dark", grille, MAX_SUGGESTIONS);
        assert_eq!(
            s.iter().map(|s| s.rowid).collect::<Vec<_>>(),
            [1, 3],
            "« Dark Magician » avant « Buster Blader, the Dark Destroyer »"
        );
    }

    /// Un numéro qui existe en sept raretés ne remplit pas la liste : une seule
    /// ligne le représente.
    #[test]
    fn les_raretes_d_un_meme_numero_ne_sont_proposees_qu_une_fois() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let s = suggestions(&jeu_recherche(), "magician", grille, MAX_SUGGESTIONS);
        assert_eq!(s.len(), 1, "une seule proposition pour RA02-EN001");
        assert_eq!(s[0].rarete, "Ultra Rare", "la première ligne du classeur");
    }

    #[test]
    fn chaque_proposition_porte_son_feuillet() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let mut cartes = jeu_recherche();
        // On pousse « Rescue Cat » au rang 20, donc sur le feuillet 2.
        for i in 0..20 {
            cartes.insert(3, carte(100 + i, "Bourrage", "SET-EN999", "Common", 0));
        }
        let s = suggestions(&cartes, "rescue", grille, MAX_SUGGESTIONS);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].position, 23);
        assert_eq!(s[0].feuillet, feuillet_de(23, grille));
        assert_eq!(s[0].feuillet, 1, "rang 23 → page 2 → feuillet 1");
    }

    #[test]
    fn un_terme_vide_ou_sans_correspondance_ne_propose_rien() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        assert!(suggestions(&jeu_recherche(), "", grille, MAX_SUGGESTIONS).is_empty());
        assert!(suggestions(&jeu_recherche(), "   ", grille, MAX_SUGGESTIONS).is_empty());
        assert!(suggestions(&jeu_recherche(), "zzzz", grille, MAX_SUGGESTIONS).is_empty());
    }

    #[test]
    fn la_liste_est_bornee() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        let cartes: Vec<Carte> = (0..50)
            .map(|i| {
                carte(
                    i64::from(i) + 1,
                    "Dark Thing",
                    &format!("SET-EN{i:03}"),
                    "Common",
                    0,
                )
            })
            .collect();
        assert_eq!(suggestions(&cartes, "dark", grille, 5).len(), 5);
        assert_eq!(
            suggestions(&cartes, "dark", grille, MAX_SUGGESTIONS).len(),
            MAX_SUGGESTIONS
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Les écarts de création
    // ─────────────────────────────────────────────────────────────────────

    fn fantome(rarete: &str) -> crate::raretes::Fantome {
        crate::raretes::Fantome {
            set_code: "CH01-EN001".to_owned(),
            name: "Blue-Eyes White Dragon".to_owned(),
            rarete: rarete.to_owned(),
            image_id: Some(4_007),
            jumelles: vec!["Ultra Rare".to_owned()],
        }
    }

    /// Un classeur neuf n'a rien à signaler — et ne montre donc rien.
    #[test]
    fn un_classeur_sans_ecart_n_en_declare_aucun() {
        let (_tmp, chemin) = classeur_vide();
        assert_eq!(ecarts_du_classeur(&chemin), None);
        assert!(Ecarts::default().est_vide());
        assert_eq!(Ecarts::default().resume(), None);
    }

    /// Ce qui est consigné se relit à l'identique : c'est la seule garantie
    /// qui compte pour un journal.
    #[test]
    fn les_ecarts_consignes_se_relisent() {
        let (_tmp, chemin) = classeur_vide();
        let ecarts = Ecarts {
            quand: "2026-09-05 14:00:00".to_owned(),
            canonisees: 12,
            fantomes: vec![fantome("New"), fantome("New artwork")],
            inconnues: BTreeMap::from([("force-SMW".to_owned(), 1)]),
        };
        enregistrer_ecarts(&chemin, &ecarts).unwrap();

        assert_eq!(ecarts_du_classeur(&chemin), Some(ecarts.clone()));
        assert_eq!(ecarts.retirees(), 2);
        assert_eq!(ecarts.lignes_inconnues(), 1);
    }

    /// Recréer proprement efface le bandeau : un classeur ne doit pas traîner
    /// les anomalies d'une version précédente.
    #[test]
    fn un_bilan_vide_efface_la_cle() {
        let (_tmp, chemin) = classeur_vide();
        enregistrer_ecarts(
            &chemin,
            &Ecarts {
                fantomes: vec![fantome("New")],
                ..Ecarts::default()
            },
        )
        .unwrap();
        assert!(ecarts_du_classeur(&chemin).is_some());

        enregistrer_ecarts(&chemin, &Ecarts::default()).unwrap();
        assert_eq!(ecarts_du_classeur(&chemin), None);

        let conn = rusqlite::Connection::open(&chemin).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM meta WHERE key = ?1",
                [CLE_ECARTS],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "la clé est retirée, pas mise à vide");
    }

    /// La phrase du bandeau nomme d'abord ce qui manque au classeur.
    #[test]
    fn le_resume_dit_le_retrait_avant_le_reste() {
        let ecarts = Ecarts {
            canonisees: 12,
            fantomes: vec![fantome("New"), fantome("New artwork")],
            inconnues: BTreeMap::from([("force-SMW".to_owned(), 1)]),
            ..Ecarts::default()
        };
        let resume = ecarts.resume().unwrap();
        assert!(resume.starts_with("2 lignes écartées"), "{resume}");
        assert!(resume.contains("1 libellé non reconnu"), "{resume}");
        assert!(resume.contains("12 raretés harmonisées"), "{resume}");
    }

    /// Une harmonisation seule ne retire rien — la phrase le dit ainsi.
    #[test]
    fn une_harmonisation_seule_se_dit_sans_parler_de_retrait() {
        let ecarts = Ecarts {
            canonisees: 1,
            ..Ecarts::default()
        };
        assert_eq!(ecarts.resume().as_deref(), Some("1 rareté harmonisée"));
        assert_eq!(ecarts.retirees(), 0);
    }

    /// Un JSON qu'on ne sait plus relire n'empêche pas d'ouvrir le classeur.
    #[test]
    fn un_journal_illisible_n_empeche_pas_l_ouverture() {
        let (_tmp, chemin) = classeur_vide();
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '{ceci n''est pas du JSON')",
            [CLE_ECARTS],
        )
        .unwrap();
        drop(conn);
        assert_eq!(ecarts_du_classeur(&chemin), None);
    }
}
