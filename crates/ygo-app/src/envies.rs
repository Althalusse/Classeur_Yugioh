// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Listes d'envies Cardmarket : les cartes qui manquent, prêtes à coller.
//!
//! # Le besoin — 2026-11-05
//!
//! Savoir vite, avant d'acheter, quelles cartes on n'a pas encore. Lire la
//! page d'un vendeur depuis l'application est exclu : Cardmarket la protège
//! (Cloudflare), et ses conditions réservent la récupération des données par
//! programme à son API, fermée aux nouvelles demandes. Le problème se prend
//! donc à l'envers : l'application écrit **la liste de ce qui manque**, au
//! format qu'accepte une liste d'envies Cardmarket (« Ajouter une
//! decklist »), et c'est Cardmarket — son assistant d'achat, ses « vendeurs
//! avec le plus de cartes » — qui trouve qui les vend. Tout ce qu'il propose
//! manque à la collection, par construction.
//!
//! Aucune requête vers Cardmarket : ce module ne lit que les fichiers publics
//! déjà téléchargés (`crate::cardmarket`) et les classeurs.
//!
//! # Le format — validé sur Cardmarket le 2026-11-06
//!
//! Une carte par ligne, `1x Nom (V.n) (Extension)` : le nom et l'extension
//! tels que Cardmarket les écrit, en anglais ; `(V.n)` désigne la version, et
//! disparaît pour une carte qui n'en a qu'une dans l'extension. Les 150
//! lignes de LOCH collées le 2026-11-06 sont toutes entrées, chacune sur la
//! bonne carte et la bonne version.
//!
//! Le collage ne porte ni l'état, ni la langue, ni le prix : ils se règlent
//! ensuite sur Cardmarket, par la « Modification en masse » de la liste.
//!
//! # Ce qui s'écrit, ce qui ne s'écrit pas
//!
//! Une ligne de classeur **sans exemplaire** s'écrit quand sa version est
//! sûre : le numéro V.n que donne son offre ([`Offre::rang`]), ou la version
//! unique de la carte. Sinon — ordre des versions inconnu, carte introuvable
//! chez Cardmarket, numéro absent de la base — elle va « à vérifier », avec
//! sa raison : un numéro deviné ferait ajouter une autre version, sans un
//! mot de Cardmarket.
//!
//! Le produit, lui, n'a pas besoin d'être sûr. Une liste d'envies désigne
//! une version par son numéro, pas par son identifiant : une carte d'un set
//! saisi dans le désordre s'écrit donc, avec le haut de sa fourchette pour
//! prix de tri.
//!
//! # Une liste par classeur, ou par rareté
//!
//! Décision de l'utilisateur, 2026-11-06 : une liste par classeur ; s'il le
//! demande, une par rareté dans chaque classeur (RA02 › Secret Rare, RA02 ›
//! Collector's Rare…) — [`Regroupement`]. Pas de liste commune à plusieurs
//! classeurs, ni de « Liste 1, Liste 2 » tranchées dans le tout.
//!
//! Chaque liste va du moins cher au plus cher — la priorité d'achat —, les
//! cartes sans prix à la fin. Le prix est celui d'un exemplaire, comme
//! partout dans l'application ([`crate::cardmarket::Prix::unitaire`]).
//!
//! Une liste d'envies Cardmarket ne prend que [`ENTREES_PAR_LISTE`]
//! entrées. Le groupe n'est pas coupé pour autant : [`Groupe::tranches`] dit
//! où Cardmarket s'arrête, et l'écran le montre.
//!
//! Les lignes qui désignent la même version — trois Dragons Blancs aux Yeux
//! Bleus dans un deck LDK2 — font une seule entrée : `3x`.
//!
//! # Une langue par liste
//!
//! La langue se règle en masse, pour toute une liste. Une liste ne mélange
//! donc pas des cartes TCG voulues en français et des cartes asiatiques, qui
//! ont leurs propres produits chez Cardmarket, en une seule langue
//! (`ygo_core::cardmarket::filtre_des_annonces`). Un classeur qui en mêle
//! fait une liste par [`LangueDeLaListe`] : celle des annonces du classeur
//! pour le TCG, « indifférente » pour les cartes asiatiques et les
//! classeurs « toutes langues ».
//!
//! # Toujours à jour
//!
//! Rien n'est enregistré : les listes se calculent à la demande, depuis
//! l'état des classeurs. Une carte achetée puis ajoutée à son classeur en
//! sort d'elle-même — décision de l'utilisateur, 2026-11-06, qui a écarté un
//! repère « liste copiée le… ».

use std::collections::HashMap;

use ygo_core::cardmarket::{self as regles, LangueDesAnnonces};
use ygo_core::paths::Paths;
use ygo_sources::cardmarket::{self as fichiers, Fichier};

use crate::cardmarket::{self as cm, Cible, IndexCharge, Offre, PrixOffre, Raison};
use crate::error::{AppError, Result};

/// Une liste d'envies Cardmarket ne prend pas plus d'entrées que cela
/// (aide de Cardmarket, « Wants List » : 150 entrées par liste, 100 listes
/// par jeu). La liste LOCH du 2026-11-06 s'est arrêtée là : « Cette
/// Wantlist est pleine. »
pub const ENTREES_PAR_LISTE: usize = 150;

// ─────────────────────────────────────────────────────────────────────────────
// Ce que l'écran montre
// ─────────────────────────────────────────────────────────────────────────────

/// La langue à choisir sur Cardmarket pour toute une liste — celle de la
/// « Modification en masse ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LangueDeLaListe {
    /// Une langue précise : une carte TCG, d'un classeur qui filtre ses
    /// annonces.
    Langue(LangueDesAnnonces),
    /// Indifférente : une carte asiatique — ses produits n'ont qu'une langue
    /// —, ou un classeur réglé sur « toutes langues ».
    Indifferente,
}

impl LangueDeLaListe {
    /// La langue d'une ligne : son numéro, et la langue des annonces de son
    /// classeur.
    ///
    /// ```
    /// use ygo_app::envies::LangueDeLaListe;
    /// use ygo_core::cardmarket::LangueDesAnnonces;
    /// let fr = LangueDesAnnonces::Francais;
    /// assert_eq!(LangueDeLaListe::de("RA02-EN077", fr), LangueDeLaListe::Langue(fr));
    /// assert_eq!(LangueDeLaListe::de("LOCH-JP036", fr), LangueDeLaListe::Indifferente);
    /// assert_eq!(
    ///     LangueDeLaListe::de("RA02-EN077", LangueDesAnnonces::Toutes),
    ///     LangueDeLaListe::Indifferente
    /// );
    /// ```
    #[must_use]
    pub fn de(set_code: &str, annonces: LangueDesAnnonces) -> Self {
        if regles::filtre_des_annonces(set_code, annonces).is_some() {
            Self::Langue(annonces)
        } else {
            Self::Indifferente
        }
    }

    /// Ce qu'on choisit dans « Langues », sur Cardmarket.
    ///
    /// ```
    /// use ygo_app::envies::LangueDeLaListe;
    /// use ygo_core::cardmarket::LangueDesAnnonces;
    /// assert_eq!(LangueDeLaListe::Langue(LangueDesAnnonces::Francais).libelle(), "Français");
    /// assert_eq!(LangueDeLaListe::Indifferente.libelle(), "Indifférent");
    /// ```
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Langue(LangueDesAnnonces::Francais) => "Français",
            Self::Langue(LangueDesAnnonces::Anglais) => "Anglais",
            Self::Langue(LangueDesAnnonces::Allemand) => "Allemand",
            Self::Langue(LangueDesAnnonces::Italien) => "Italien",
            Self::Langue(LangueDesAnnonces::Espagnol) => "Espagnol",
            Self::Langue(LangueDesAnnonces::Portugais) => "Portugais",
            Self::Langue(LangueDesAnnonces::Toutes) | Self::Indifferente => "Indifférent",
        }
    }

    /// Le rang de la langue dans l'écran : les langues dans l'ordre des
    /// Options, l'indifférente à la fin.
    fn ordre(self) -> usize {
        match self {
            Self::Langue(l) => LangueDesAnnonces::toutes()
                .iter()
                .position(|x| *x == l)
                .unwrap_or(usize::MAX - 1),
            Self::Indifferente => usize::MAX,
        }
    }
}

/// Une ligne de classeur derrière une entrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origine {
    /// Le code du classeur.
    pub classeur: String,
    /// Le numéro de la carte.
    pub set_code: String,
    /// Sa version : la rareté, ou sa clé de version
    /// (`crate::cardmarket::cles_de_version`).
    pub version: String,
    /// Sa rareté, telle que le classeur l'écrit : le regroupement par
    /// rareté.
    pub rarete: String,
}

/// Une entrée de liste d'envies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entree {
    /// Combien d'exemplaires manquent : une par ligne de classeur.
    pub quantite: u32,
    /// Le nom de la carte chez Cardmarket.
    pub nom: String,
    /// Le numéro de la version, V.n — `None` pour une carte qui n'a qu'une
    /// version dans l'extension.
    pub rang: Option<u32>,
    /// Le nom de l'extension chez Cardmarket.
    pub extension: String,
    /// Le prix d'un exemplaire, en centimes, qui ordonne la liste.
    pub prix: Option<u32>,
    /// Le prix est-il celui du produit même ? Sinon, le haut de la
    /// fourchette de la carte.
    pub prix_sur: bool,
    /// Les lignes de classeur qu'elle couvre.
    pub origines: Vec<Origine>,
}

impl Entree {
    /// La ligne à coller.
    ///
    /// ```
    /// use ygo_app::envies::Entree;
    /// let mut e = Entree {
    ///     quantite: 1,
    ///     nom: "Skill Drain".into(),
    ///     rang: Some(6),
    ///     extension: "25th Anniversary Rarity Collection II".into(),
    ///     prix: None,
    ///     prix_sur: false,
    ///     origines: Vec::new(),
    /// };
    /// assert_eq!(e.ligne(), "1x Skill Drain (V.6) (25th Anniversary Rarity Collection II)");
    /// e.quantite = 3;
    /// e.rang = None;
    /// assert_eq!(e.ligne(), "3x Skill Drain (25th Anniversary Rarity Collection II)");
    /// ```
    #[must_use]
    pub fn ligne(&self) -> String {
        let version = self.rang.map(|n| format!(" (V.{n})")).unwrap_or_default();
        format!(
            "{}x {}{version} ({})",
            self.quantite, self.nom, self.extension
        )
    }
}

/// Comment les entrées se regroupent — au choix de l'utilisateur,
/// 2026-11-06 : une liste par classeur, ou une par rareté dans chaque
/// classeur (RA02 › Secret Rare, RA02 › Collector's Rare…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Regroupement {
    /// Une liste par classeur.
    #[default]
    ParClasseur,
    /// Une liste par rareté, dans chaque classeur.
    ParRarete,
}

impl Regroupement {
    /// Le libellé du choix.
    ///
    /// ```
    /// use ygo_app::envies::Regroupement;
    /// assert_eq!(Regroupement::ParRarete.libelle(), "Par rareté");
    /// ```
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::ParClasseur => "Par classeur",
            Self::ParRarete => "Par rareté",
        }
    }
}

/// Le texte à coller pour des entrées : une par ligne.
#[must_use]
pub fn texte_de(entrees: &[Entree]) -> String {
    entrees.iter().map(|e| e.ligne() + "\n").collect()
}

/// Une liste d'un classeur — ou d'une rareté d'un classeur —, d'une seule
/// langue, de la moins chère à la plus chère.
///
/// Elle n'est pas coupée : au-delà de [`ENTREES_PAR_LISTE`] entrées,
/// Cardmarket s'arrête, et la suite va dans une autre liste sur Cardmarket.
/// [`Groupe::tranches`] dit où.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Groupe {
    /// Le code du classeur.
    pub classeur: String,
    /// La rareté, quand on regroupe par rareté.
    pub rarete: Option<String>,
    /// La langue à régler sur Cardmarket.
    pub langue: LangueDeLaListe,
    /// Le nom à donner à la liste sur Cardmarket.
    pub nom: String,
    /// Les entrées, de la moins chère à la plus chère.
    pub entrees: Vec<Entree>,
}

impl Groupe {
    /// Combien de listes Cardmarket il faut : une par tranche de
    /// [`ENTREES_PAR_LISTE`] entrées.
    #[must_use]
    pub fn listes(&self) -> usize {
        self.entrees.len().div_ceil(ENTREES_PAR_LISTE)
    }

    /// Les entrées, par [`ENTREES_PAR_LISTE`] : ce que prend chaque liste
    /// Cardmarket.
    pub fn tranches(&self) -> std::slice::Chunks<'_, Entree> {
        self.entrees.chunks(ENTREES_PAR_LISTE)
    }

    /// Le nom de la liste Cardmarket de la tranche `i` (à partir de 0) : le
    /// nom du groupe, suivi de son numéro quand il en faut plusieurs.
    ///
    /// ```
    /// use ygo_app::envies::{Groupe, LangueDeLaListe};
    /// let mut g = Groupe {
    ///     classeur: "RA02".into(),
    ///     rarete: None,
    ///     langue: LangueDeLaListe::Indifferente,
    ///     nom: "RA02".into(),
    ///     entrees: Vec::new(),
    /// };
    /// assert_eq!(g.nom_de_tranche(0), "RA02");
    /// ```
    #[must_use]
    pub fn nom_de_tranche(&self, i: usize) -> String {
        let n = self.listes();
        if n > 1 {
            format!("{} {}/{n}", self.nom, i + 1)
        } else {
            self.nom.clone()
        }
    }

    /// Le texte à coller de la tranche `i` (à partir de 0).
    #[must_use]
    pub fn texte_de_tranche(&self, i: usize) -> String {
        self.tranches().nth(i).map(texte_de).unwrap_or_default()
    }

    /// Le texte de toutes les entrées.
    #[must_use]
    pub fn texte(&self) -> String {
        texte_de(&self.entrees)
    }

    /// Le nombre d'exemplaires voulus.
    #[must_use]
    pub fn cartes(&self) -> u32 {
        self.entrees.iter().map(|e| e.quantite).sum()
    }

    /// La valeur des exemplaires dont on connaît le prix, en centimes.
    #[must_use]
    pub fn valeur(&self) -> u64 {
        self.entrees
            .iter()
            .filter_map(|e| e.prix.map(|p| u64::from(p) * u64::from(e.quantite)))
            .sum()
    }

    /// Les entrées sans prix connu.
    #[must_use]
    pub fn sans_prix(&self) -> usize {
        self.entrees.iter().filter(|e| e.prix.is_none()).count()
    }

    /// Le prix le plus bas et le plus haut des entrées qui en ont un.
    #[must_use]
    pub fn extremes(&self) -> Option<(u32, u32)> {
        let prix = self.entrees.iter().filter_map(|e| e.prix);
        let bas = prix.clone().min()?;
        let haut = prix.max()?;
        Some((bas, haut))
    }
}

/// Pourquoi une ligne ne s'écrit pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motif {
    /// L'offre ne donne pas le numéro de la version (sa raison dit pourquoi).
    Version(Raison),
    /// L'extension Cardmarket de la carte n'a pas de nom connu : une liste
    /// d'envies la nomme, sans quoi « V.n » ne désigne rien.
    ExtensionSansNom,
}

impl Motif {
    /// Une phrase pour l'utilisateur.
    #[must_use]
    pub fn explication(self) -> String {
        match self {
            Self::Version(raison) => raison.explication(),
            Self::ExtensionSansNom => "nom de l'extension Cardmarket inconnu".to_owned(),
        }
    }
}

/// Une ligne qui manque, mais qu'on ne peut pas écrire sans risque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AVerifier {
    /// La ligne.
    pub origine: Origine,
    /// Son nom, tel que le classeur l'écrit.
    pub nom: String,
    /// Son offre — le lien de recherche à proposer.
    pub offre: Offre,
    /// Pourquoi elle n'est pas dans une liste.
    pub motif: Motif,
}

/// Les listes d'envies d'un ou de plusieurs classeurs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Envies {
    /// Les classeurs lus, dans l'ordre demandé.
    pub classeurs: Vec<String>,
    /// Les classeurs qui n'ont pas pu l'être.
    pub illisibles: Vec<String>,
    /// Les lignes sans exemplaire : de quoi regrouper autrement sans tout
    /// relire.
    pub lignes: Vec<Manquante>,
    /// Le regroupement des [`Envies::groupes`].
    pub regroupement: Regroupement,
    /// Les listes, classeur par classeur, chacune de la moins chère à la
    /// plus chère.
    pub groupes: Vec<Groupe>,
    /// Ce qui manque sans pouvoir s'écrire.
    pub a_verifier: Vec<AVerifier>,
    /// Date du guide des prix.
    pub prix_du: String,
}

impl Envies {
    /// Regroupe les lignes autrement — rien à relire.
    pub fn regrouper(&mut self, regroupement: Regroupement) {
        let (groupes, a_verifier) = classer(self.lignes.clone(), regroupement);
        self.regroupement = regroupement;
        self.groupes = groupes;
        self.a_verifier = a_verifier;
    }

    /// Les lignes sans exemplaire.
    #[must_use]
    pub fn manquantes(&self) -> usize {
        self.lignes.len()
    }

    /// Les entrées de toutes les listes.
    #[must_use]
    pub fn entrees(&self) -> usize {
        self.groupes.iter().map(|g| g.entrees.len()).sum()
    }

    /// Les exemplaires de toutes les listes.
    #[must_use]
    pub fn cartes(&self) -> u32 {
        self.groupes.iter().map(Groupe::cartes).sum()
    }

    /// Le nombre de listes Cardmarket qu'il faut en tout.
    #[must_use]
    pub fn listes(&self) -> usize {
        self.groupes.iter().map(Groupe::listes).sum()
    }

    /// Les listes d'un classeur.
    pub fn du_classeur<'a>(&'a self, code: &'a str) -> impl Iterator<Item = &'a Groupe> + 'a {
        self.groupes.iter().filter(move |g| g.classeur == code)
    }

    /// Ce qui reste à vérifier dans un classeur.
    pub fn a_verifier_du_classeur<'a>(
        &'a self,
        code: &'a str,
    ) -> impl Iterator<Item = &'a AVerifier> + 'a {
        self.a_verifier
            .iter()
            .filter(move |a| a.origine.classeur == code)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Le classement — fonctions pures
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne sans exemplaire, avant le classement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manquante {
    /// La ligne.
    pub origine: Origine,
    /// Son nom, tel que le classeur l'écrit — le repli quand Cardmarket ne
    /// donne pas le sien.
    pub nom_classeur: String,
    /// Le nom du produit chez Cardmarket.
    pub nom_cardmarket: Option<String>,
    /// La langue de sa liste.
    pub langue: LangueDeLaListe,
    /// Son offre.
    pub offre: Offre,
}

/// La version à écrire pour une offre : `Some(Some(n))` pour « V.n »,
/// `Some(None)` pour une carte à version unique, `None` quand la version
/// n'est pas sûre.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// use ygo_app::envies::version_a_ecrire;
/// let o = |cible, version: Option<&str>, raison| Offre {
///     cible,
///     version: version.map(str::to_owned),
///     prix: PrixOffre::Aucun,
///     raison,
///     extension: None,
///     prix_du: String::new(),
/// };
/// let recherche = Cible::Recherche { nom: "X".into(), id_extension: Some(1) };
/// assert_eq!(version_a_ecrire(&o(Cible::Produit(1), None, Raison::VersionUnique)), Some(None));
/// // La seule fiche en vente (RA05 « Tri-Horned Dragon ») : pas de V.n non plus.
/// assert_eq!(version_a_ecrire(&o(Cible::Produit(4), None, Raison::SeuleFicheEnVente)), Some(None));
/// assert_eq!(
///     version_a_ecrire(&o(recherche.clone(), Some("V.4 · Ultra Rare"), Raison::SaisieMelangee)),
///     Some(Some(4))
/// );
/// assert_eq!(version_a_ecrire(&o(recherche, None, Raison::CarteAbsente)), None);
/// // Le produit sûr mais le numéro tu (une carte créée à rebours) : rien.
/// assert_eq!(version_a_ecrire(&o(Cible::Produit(7), None, Raison::SensDeLaCarte)), None);
/// ```
#[must_use]
pub fn version_a_ecrire(offre: &Offre) -> Option<Option<u32>> {
    if matches!(
        offre.raison,
        Raison::VersionUnique | Raison::SeuleFicheEnVente
    ) && offre.est_directe()
    {
        return Some(None);
    }
    offre.rang().map(Some)
}

/// Le prix qui ordonne la liste, et s'il est celui du produit : le prix d'un
/// exemplaire quand le produit est sûr, le haut de la fourchette sinon.
#[must_use]
pub fn prix_de_tri(offre: &Offre) -> (Option<u32>, bool) {
    match offre.prix {
        PrixOffre::Exact(p) => (p.unitaire(), true),
        PrixOffre::Fourchette { max, .. } => (Some(max), false),
        PrixOffre::Aucun => (None, false),
    }
}

/// Le nom d'un produit tel qu'il s'écrit dans une liste : le fichier public
/// double les guillemets (`""A"" Cell Breeding Device`), la carte n'en a
/// qu'un.
///
/// ```
/// use ygo_app::envies::nom_de_produit;
/// assert_eq!(nom_de_produit("\"\"A\"\" Cell Breeding Device"), "\"A\" Cell Breeding Device");
/// assert_eq!(nom_de_produit(" Skill Drain "), "Skill Drain");
/// ```
#[must_use]
pub fn nom_de_produit(brut: &str) -> String {
    brut.replace("\"\"", "\"").trim().to_owned()
}

/// Classe les lignes manquantes : les listes, classeur par classeur — et
/// rareté par rareté, si on le demande —, et ce qui reste à vérifier.
///
/// Les classeurs gardent l'ordre de leur première ligne ; dans un classeur,
/// les raretés suivent l'ordre du référentiel
/// ([`crate::statistiques::rang_rarete`]), les langues celui des Options.
#[must_use]
pub fn classer(
    manquantes: Vec<Manquante>,
    regroupement: Regroupement,
) -> (Vec<Groupe>, Vec<AVerifier>) {
    /// Ce qui fait un seul groupe.
    type CleDeGroupe = (String, Option<String>, LangueDeLaListe);
    /// Ce qui fait une seule entrée, dans un groupe.
    type CleDEntree = (String, Option<u32>, String);
    struct EnCours {
        cle: CleDeGroupe,
        entrees: Vec<Entree>,
        place: HashMap<CleDEntree, usize>,
    }

    let mut ordre_des_classeurs: Vec<String> = Vec::new();
    let mut groupes: Vec<EnCours> = Vec::new();
    let mut place_du_groupe: HashMap<CleDeGroupe, usize> = HashMap::new();
    let mut a_verifier = Vec::new();
    for m in manquantes {
        if !ordre_des_classeurs.contains(&m.origine.classeur) {
            ordre_des_classeurs.push(m.origine.classeur.clone());
        }
        let Some(rang) = version_a_ecrire(&m.offre) else {
            let motif = Motif::Version(m.offre.raison);
            a_verifier.push(AVerifier {
                origine: m.origine,
                nom: m.nom_classeur,
                offre: m.offre,
                motif,
            });
            continue;
        };
        let Some(extension) = m.offre.extension.clone().filter(|e| !e.trim().is_empty()) else {
            a_verifier.push(AVerifier {
                origine: m.origine,
                nom: m.nom_classeur,
                offre: m.offre,
                motif: Motif::ExtensionSansNom,
            });
            continue;
        };
        let nom = m
            .nom_cardmarket
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| m.nom_classeur.clone());
        let rarete = match regroupement {
            Regroupement::ParClasseur => None,
            Regroupement::ParRarete => Some(libelle_de_rarete(&m.origine.rarete)),
        };
        let cle_de_groupe: CleDeGroupe = (m.origine.classeur.clone(), rarete, m.langue);
        let i = *place_du_groupe
            .entry(cle_de_groupe.clone())
            .or_insert_with(|| {
                groupes.push(EnCours {
                    cle: cle_de_groupe,
                    entrees: Vec::new(),
                    place: HashMap::new(),
                });
                groupes.len() - 1
            });
        let Some(groupe) = groupes.get_mut(i) else {
            continue;
        };
        let cle: CleDEntree = (nom.clone(), rang, extension.clone());
        if let Some(e) = groupe
            .place
            .get(&cle)
            .and_then(|j| groupe.entrees.get_mut(*j))
        {
            e.quantite += 1;
            e.origines.push(m.origine);
            continue;
        }
        let (prix, prix_sur) = prix_de_tri(&m.offre);
        groupe.place.insert(cle, groupe.entrees.len());
        groupe.entrees.push(Entree {
            quantite: 1,
            nom,
            rang,
            extension,
            prix,
            prix_sur,
            origines: vec![m.origine],
        });
    }

    let rang = |code: &str| {
        ordre_des_classeurs
            .iter()
            .position(|c| c == code)
            .unwrap_or(usize::MAX)
    };
    groupes.sort_by(|a, b| {
        let (ca, ra, la) = &a.cle;
        let (cb, rb, lb) = &b.cle;
        let rarete = |r: &Option<String>| r.as_deref().map_or(0, crate::statistiques::rang_rarete);
        (rang(ca), rarete(ra), ra, la.ordre()).cmp(&(rang(cb), rarete(rb), rb, lb.ordre()))
    });
    // Le nom porte la langue quand un classeur en mêle plusieurs.
    let mut langues_du_classeur: HashMap<&str, Vec<LangueDeLaListe>> = HashMap::new();
    for g in &groupes {
        let langues = langues_du_classeur.entry(g.cle.0.as_str()).or_default();
        if !langues.contains(&g.cle.2) {
            langues.push(g.cle.2);
        }
    }
    let plusieurs: Vec<String> = langues_du_classeur
        .into_iter()
        .filter(|(_, l)| l.len() > 1)
        .map(|(c, _)| c.to_owned())
        .collect();
    let groupes = groupes
        .into_iter()
        .map(|g| {
            let (classeur, rarete, langue) = g.cle;
            let mut entrees = g.entrees;
            // Du moins cher au plus cher, les prix inconnus à la fin ; à prix
            // égal, l'ordre de Cardmarket — extension, nom, version.
            entrees.sort_by(|a, b| {
                (a.prix.is_none(), a.prix, &a.extension, &a.nom, a.rang).cmp(&(
                    b.prix.is_none(),
                    b.prix,
                    &b.extension,
                    &b.nom,
                    b.rang,
                ))
            });
            let mut nom = classeur.clone();
            if let Some(r) = &rarete {
                nom.push_str(" · ");
                nom.push_str(r);
            }
            if plusieurs.contains(&classeur) {
                nom.push_str(" · ");
                nom.push_str(langue.libelle());
            }
            Groupe {
                classeur,
                rarete,
                langue,
                nom,
                entrees,
            }
        })
        .collect();
    a_verifier.sort_by(|a, b| {
        (
            rang(&a.origine.classeur),
            &a.origine.set_code,
            &a.origine.version,
        )
            .cmp(&(
                rang(&b.origine.classeur),
                &b.origine.set_code,
                &b.origine.version,
            ))
    });
    (groupes, a_verifier)
}

/// La rareté d'une ligne, pour titrer son groupe.
fn libelle_de_rarete(rarete: &str) -> String {
    let r = rarete.trim();
    if r.is_empty() {
        "Sans rareté".to_owned()
    } else {
        r.to_owned()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// La lecture
// ─────────────────────────────────────────────────────────────────────────────

/// Les noms des produits chez Cardmarket, lus dans le catalogue public.
#[derive(Debug, Default)]
struct Noms {
    par_id: HashMap<i64, String>,
}

impl Noms {
    /// Le catalogue des cartes de l'installation. Illisible, il ne prive que
    /// des noms exacts : le nom du classeur prend le relais.
    fn lire(paths: &Paths) -> Self {
        match fichiers::lire_catalogue(&cm::chemin_fichier(paths, Fichier::Cartes)) {
            Ok(catalogue) => Self {
                par_id: catalogue
                    .products
                    .into_iter()
                    .map(|p| (p.id, nom_de_produit(&p.name)))
                    .collect(),
            },
            Err(e) => {
                tracing::warn!(erreur = %e, "envies : catalogue Cardmarket illisible");
                Self::default()
            }
        }
    }

    /// Le nom Cardmarket de la carte d'une offre : celui de son produit ; à
    /// défaut — le produit n'est pas sûr —, celui de n'importe quelle version
    /// de la carte dans l'extension, qui portent toutes le même.
    fn de(&self, offre: &Offre, index: &IndexCharge) -> Option<String> {
        let id = match &offre.cible {
            Cible::Produit(id) => Some(*id),
            Cible::Recherche {
                nom,
                id_extension: Some(ext),
            } => {
                let cle = regles::normaliser_nom(nom);
                index
                    .index
                    .extensions
                    .iter()
                    .filter(|e| e.id == *ext)
                    .find_map(|e| e.cartes.get(&cle))
                    .and_then(|v| v.ids.first().or_else(|| v.ajouts.first()).copied())
            }
            Cible::Recherche { .. } | Cible::Numero(_) => None,
        }?;
        self.par_id.get(&id).cloned()
    }
}

/// Les listes d'envies des classeurs donnés — `None` tant qu'il n'y a pas
/// d'index Cardmarket ou de base des cartes.
///
/// Toutes les lignes de chaque classeur passent par leur offre, celle que
/// montre leur fiche (`crate::cardmarket::offres_du_classeur`) : quelques
/// dixièmes de seconde pour toute une collection, plus sur un disque froid
/// — à faire hors du fil de l'interface. Un classeur illisible est sauté, et
/// nommé dans [`Envies::illisibles`].
///
/// # Errors
///
/// Si `cardinfo.db` ne s'ouvre pas.
pub fn preparer(
    paths: &Paths,
    codes: &[String],
    regroupement: Regroupement,
) -> Result<Option<Envies>> {
    let Some(index) = cm::charge(paths) else {
        return Ok(None);
    };
    if !paths.cardinfo_db().is_file() {
        return Ok(None);
    }
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let noms = Noms::lire(paths);
    let annonces_des_options =
        ygo_core::config::Config::charger(paths.app_config()).annonces_cardmarket();
    let mut envies = Envies {
        prix_du: index.index.empreinte.prix.clone(),
        ..Envies::default()
    };
    let mut manquantes = Vec::new();
    for code in codes {
        let offres = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(code))
            .map_err(AppError::from)
            .and_then(|conn| cm::offres_du_classeur(&index, &cardinfo, &conn, false));
        let offres = match offres {
            Ok(offres) => offres,
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "envies : classeur illisible");
                envies.illisibles.push(code.clone());
                continue;
            }
        };
        let annonces =
            crate::classeur::annonces_du_classeur(&paths.classeur_db(code), annonces_des_options);
        envies.classeurs.push(code.clone());
        for l in offres.into_iter().filter(|l| l.quantite <= 0) {
            manquantes.push(Manquante {
                langue: LangueDeLaListe::de(&l.set_code, annonces),
                nom_cardmarket: noms.de(&l.offre, &index),
                origine: Origine {
                    classeur: code.clone(),
                    set_code: l.set_code,
                    version: l.version,
                    rarete: l.rarete,
                },
                nom_classeur: l.nom,
                offre: l.offre,
            });
        }
    }
    envies.lignes = manquantes;
    envies.regrouper(regroupement);
    Ok(Some(envies))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use super::*;
    use crate::cardmarket::Prix;

    fn offre(cible: Cible, version: Option<&str>, prix: PrixOffre, raison: Raison) -> Offre {
        Offre {
            cible,
            version: version.map(str::to_owned),
            prix,
            raison,
            extension: Some("Set A".to_owned()),
            prix_du: String::new(),
        }
    }

    fn exact(centimes: u32) -> PrixOffre {
        PrixOffre::Exact(Prix {
            tendance: Some(centimes),
            bas: Some(centimes),
            moyenne30: Some(centimes),
        })
    }

    fn manquante(
        code: &str,
        nom: &str,
        rang: u32,
        prix: PrixOffre,
        langue: LangueDeLaListe,
    ) -> Manquante {
        Manquante {
            origine: Origine {
                classeur: "AAA".into(),
                set_code: code.into(),
                version: "Secret Rare".into(),
                rarete: "Secret Rare".into(),
            },
            nom_classeur: nom.into(),
            nom_cardmarket: Some(nom.into()),
            langue,
            offre: offre(
                Cible::Produit(i64::from(rang)),
                Some(&format!("V.{rang} · Secret Rare")),
                prix,
                Raison::Ordonnee,
            ),
        }
    }

    const FR: LangueDeLaListe = LangueDeLaListe::Langue(LangueDesAnnonces::Francais);

    #[test]
    fn les_listes_vont_du_moins_cher_au_plus_cher_sans_prix_a_la_fin() {
        let (listes, a_verifier) = classer(
            vec![
                manquante("AAA-EN003", "Gamma", 1, exact(900), FR),
                manquante("AAA-EN001", "Alpha", 1, PrixOffre::Aucun, FR),
                manquante("AAA-EN002", "Beta", 1, exact(5), FR),
            ],
            Regroupement::ParClasseur,
        );
        assert!(a_verifier.is_empty());
        assert_eq!(listes.len(), 1);
        let noms: Vec<&str> = listes[0].entrees.iter().map(|e| e.nom.as_str()).collect();
        assert_eq!(noms, ["Beta", "Gamma", "Alpha"]);
        assert_eq!(listes[0].extremes(), Some((5, 900)));
        assert_eq!(listes[0].sans_prix(), 1);
        assert_eq!(listes[0].valeur(), 905);
        assert_eq!(
            listes[0].texte(),
            "1x Beta (V.1) (Set A)\n1x Gamma (V.1) (Set A)\n1x Alpha (V.1) (Set A)\n"
        );
    }

    #[test]
    fn la_meme_version_fait_une_seule_entree() {
        let mut deux = manquante("LDK2-ENK01", "Blue-Eyes White Dragon", 1, exact(50), FR);
        deux.origine.version = "Common".into();
        let (listes, _) = classer(
            vec![deux.clone(), deux.clone(), deux],
            Regroupement::ParClasseur,
        );
        assert_eq!(listes[0].entrees.len(), 1);
        let e = &listes[0].entrees[0];
        assert_eq!(e.quantite, 3);
        assert_eq!(e.origines.len(), 3);
        assert_eq!(e.ligne(), "3x Blue-Eyes White Dragon (V.1) (Set A)");
        assert_eq!(listes[0].cartes(), 3);
        assert_eq!(listes[0].valeur(), 150);
    }

    #[test]
    fn un_classeur_n_est_pas_coupe_mais_dit_ou_cardmarket_s_arrete() {
        let manquantes: Vec<Manquante> = (1..=ENTREES_PAR_LISTE as u32 + 2)
            .map(|i| {
                manquante(
                    &format!("AAA-EN{i:03}"),
                    &format!("Carte {i:03}"),
                    1,
                    exact(i),
                    FR,
                )
            })
            .collect();
        let (groupes, _) = classer(manquantes, Regroupement::ParClasseur);
        // Une seule liste pour le classeur, pas coupée…
        assert_eq!(groupes.len(), 1);
        let g = &groupes[0];
        assert_eq!(g.entrees.len(), ENTREES_PAR_LISTE + 2);
        // … mais Cardmarket s'arrête à 150 : deux listes sur le site.
        assert_eq!(g.listes(), 2);
        let tailles: Vec<usize> = g.tranches().map(<[Entree]>::len).collect();
        assert_eq!(tailles, [ENTREES_PAR_LISTE, 2]);
        assert_eq!(g.nom_de_tranche(0), "AAA 1/2");
        assert_eq!(g.nom_de_tranche(1), "AAA 2/2");
        // Les moins chères d'abord.
        assert_eq!(
            g.texte_de_tranche(1),
            "1x Carte 151 (V.1) (Set A)\n1x Carte 152 (V.1) (Set A)\n"
        );
        assert_eq!(g.texte_de_tranche(2), "");
        assert_eq!(g.texte().lines().count(), ENTREES_PAR_LISTE + 2);
    }

    #[test]
    fn une_liste_par_langue() {
        let (listes, _) = classer(
            vec![
                manquante(
                    "LOCH-JP036",
                    "Slash Draw",
                    2,
                    exact(10),
                    LangueDeLaListe::Indifferente,
                ),
                manquante("RA02-EN077", "Skill Drain", 6, exact(50), FR),
            ],
            Regroupement::ParClasseur,
        );
        assert_eq!(listes.len(), 2);
        assert_eq!(listes[0].langue, FR, "la langue des Options d'abord");
        assert_eq!(listes[1].langue, LangueDeLaListe::Indifferente);
        // Le classeur en mêle deux : le nom le dit.
        assert_eq!(listes[0].nom, "AAA · Français");
        assert_eq!(listes[1].nom_de_tranche(0), "AAA · Indifférent");
    }

    #[test]
    fn une_liste_par_classeur_ou_par_rarete_dans_chaque_classeur() {
        let carte = |classeur: &str, code: &str, rarete: &str, prix| {
            let mut m = manquante(code, code, 1, exact(prix), FR);
            m.origine.classeur = classeur.into();
            m.origine.rarete = rarete.into();
            m
        };
        let lignes = vec![
            carte("RA02", "RA02-EN001", "Secret Rare", 300),
            carte("LOCH", "LOCH-EN001", "Common", 5),
            carte("RA02", "RA02-EN002", "Common", 900),
            carte("RA02", "RA02-EN003", "", 1),
        ];
        let (groupes, _) = classer(lignes.clone(), Regroupement::ParClasseur);
        let noms: Vec<&str> = groupes.iter().map(|g| g.nom.as_str()).collect();
        assert_eq!(noms, ["RA02", "LOCH"], "l'ordre des classeurs demandés");
        let ra02: Vec<&str> = groupes[0].entrees.iter().map(|e| e.nom.as_str()).collect();
        assert_eq!(ra02, ["RA02-EN003", "RA02-EN001", "RA02-EN002"]);

        let (groupes, _) = classer(lignes.clone(), Regroupement::ParRarete);
        let noms: Vec<&str> = groupes.iter().map(|g| g.nom.as_str()).collect();
        assert_eq!(
            noms,
            [
                "RA02 · Common",
                "RA02 · Secret Rare",
                "RA02 · Sans rareté",
                "LOCH · Common"
            ],
            "les raretés dans l'ordre du référentiel, l'inconnue à la fin"
        );
        assert_eq!(groupes[1].rarete.as_deref(), Some("Secret Rare"));

        // Regrouper ne relit rien.
        let mut envies = Envies {
            lignes,
            ..Envies::default()
        };
        envies.regrouper(Regroupement::ParRarete);
        assert_eq!(envies.groupes.len(), 4);
        assert_eq!(envies.du_classeur("RA02").count(), 3);
        assert_eq!(envies.listes(), 4);
        envies.regrouper(Regroupement::ParClasseur);
        assert_eq!(envies.groupes.len(), 2);
        assert_eq!((envies.entrees(), envies.cartes()), (4, 4));
        assert_eq!(envies.manquantes(), 4);
    }

    #[test]
    fn une_version_incertaine_ou_une_extension_sans_nom_va_a_verifier() {
        let mut incertaine = manquante("AAA-EN009", "Iota", 1, exact(10), FR);
        incertaine.offre.version = None;
        incertaine.offre.raison = Raison::PlusieursVersionsDeLaRarete;
        let mut sans_nom = manquante("AAA-EN008", "Theta", 1, exact(10), FR);
        sans_nom.offre.extension = None;
        let ecrite = manquante("AAA-EN007", "Eta", 2, exact(10), FR);
        let (listes, a_verifier) = classer(
            vec![incertaine, sans_nom, ecrite],
            Regroupement::ParClasseur,
        );
        assert_eq!(listes[0].entrees.len(), 1);
        assert_eq!(listes[0].entrees[0].nom, "Eta");
        assert_eq!(a_verifier.len(), 2);
        // Dans l'ordre du classeur : numéro, puis version.
        assert_eq!(a_verifier[0].origine.set_code, "AAA-EN008");
        assert_eq!(a_verifier[0].motif, Motif::ExtensionSansNom);
        assert_eq!(
            a_verifier[1].motif,
            Motif::Version(Raison::PlusieursVersionsDeLaRarete)
        );
        assert!(a_verifier[1]
            .motif
            .explication()
            .contains("plusieurs versions"));
    }

    #[test]
    fn la_version_unique_s_ecrit_sans_numero_et_le_nom_cardmarket_prime() {
        let mut unique = manquante("SDK-001", "Blue-Eyes White Dragon", 1, exact(300), FR);
        unique.offre.version = None;
        unique.offre.raison = Raison::VersionUnique;
        unique.nom_classeur = "Blue Eyes White Dragon".into();
        let mut sans_nom = manquante("AAA-EN010", "Kappa", 1, exact(5), FR);
        sans_nom.nom_cardmarket = None;
        let (listes, _) = classer(vec![unique, sans_nom], Regroupement::ParClasseur);
        let lignes: Vec<String> = listes[0].entrees.iter().map(Entree::ligne).collect();
        assert_eq!(
            lignes,
            [
                "1x Kappa (V.1) (Set A)",
                "1x Blue-Eyes White Dragon (Set A)"
            ]
        );
    }

    #[test]
    fn un_produit_incertain_se_trie_au_haut_de_sa_fourchette() {
        let mut melangee = manquante("AAA-EN011", "Lambda", 3, exact(0), FR);
        melangee.offre.cible = Cible::Recherche {
            nom: "Lambda".into(),
            id_extension: Some(10),
        };
        melangee.offre.raison = Raison::SaisieMelangee;
        melangee.offre.prix = PrixOffre::Fourchette { min: 10, max: 700 };
        let (listes, _) = classer(vec![melangee], Regroupement::ParClasseur);
        let e = &listes[0].entrees[0];
        assert_eq!(e.ligne(), "1x Lambda (V.3) (Set A)");
        assert_eq!((e.prix, e.prix_sur), (Some(700), false));
    }

    /// De bout en bout, sur une installation temporaire : l'index, la base
    /// et un classeur, comme l'écran les lit.
    #[test]
    fn les_envies_d_un_classeur_reel() {
        use crate::cardmarket::tests::{cardinfo_de_test, index_de_test};
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dir.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        cm::ecrire(&paths, &index_de_test()).unwrap();
        // Le catalogue des noms : Cardmarket écrit « Alpha » autrement que
        // le classeur, entre guillemets doublés.
        let catalogue = cm::chemin_fichier(&paths, Fichier::Cartes);
        std::fs::create_dir_all(catalogue.parent().unwrap()).unwrap();
        std::fs::write(
            &catalogue,
            r#"{"createdAt":"2026-11-06","products":[
                {"idProduct":100,"name":"\"\"Alpha\"\"","idExpansion":10},
                {"idProduct":101,"name":"\"\"Alpha\"\"","idExpansion":10}]}"#,
        )
        .unwrap();
        cardinfo_de_test()
            .execute(
                "VACUUM INTO ?1",
                [paths.cardinfo_db().to_string_lossy().into_owned()],
            )
            .unwrap();
        std::fs::create_dir_all(paths.dossier_classeur("AAA")).unwrap();
        let classeur = rusqlite::Connection::open(paths.classeur_db("AAA")).unwrap();
        classeur
            .execute_batch(
                "CREATE TABLE cards (name TEXT, set_code TEXT, rarity TEXT, quantite INTEGER,
                                     sort_order INTEGER);
                 INSERT INTO cards VALUES
                     ('Alpha', 'AAA-EN001', 'Secret Rare', 2, 1),
                     ('Alpha', 'AAA-EN001', 'Ultra Rare', 0, 2),
                     ('Omega', 'AAA-EN099', 'Common', 0, 3),
                     ('Zed', 'ZZZ-JP001', 'Rare', 0, 4);",
            )
            .unwrap();
        drop(classeur);

        let envies = preparer(
            &paths,
            &["AAA".to_owned(), "ABSENT".to_owned()],
            Regroupement::ParClasseur,
        )
        .unwrap()
        .unwrap();
        assert_eq!(envies.classeurs, ["AAA"]);
        assert_eq!(envies.illisibles, ["ABSENT"]);
        assert_eq!(envies.manquantes(), 3, "la Secret possédée n'y est pas");
        // L'Ultra manquante, V.1 : le nom du produit, l'extension nommée.
        assert_eq!(envies.groupes.len(), 1);
        assert_eq!(envies.groupes[0].langue, FR, "le défaut des Options");
        assert_eq!(envies.groupes[0].texte(), "1x \"Alpha\" (V.1) (Set A)\n");
        // Tendance 1,00 €, plus bas 0,50 €, pas de moyenne : 0,75 €.
        assert_eq!(envies.groupes[0].entrees[0].prix, Some(75));
        assert!(envies.groupes[0].entrees[0].prix_sur);
        // Omega, absente de l'extension ; Zed, d'un set non rapproché.
        let motifs: Vec<Motif> = envies.a_verifier.iter().map(|a| a.motif).collect();
        assert_eq!(
            motifs,
            [
                Motif::Version(Raison::CarteAbsente),
                Motif::Version(Raison::ExtensionInconnue)
            ]
        );
        assert_eq!(envies.cartes(), 1);
        assert_eq!(envies.groupes[0].nom, "AAA");
        assert_eq!(envies.a_verifier_du_classeur("AAA").count(), 2);
    }

    #[test]
    fn sans_index_rien_n_est_prepare() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dir.path());
        assert_eq!(
            preparer(&paths, &["AAA".to_owned()], Regroupement::ParClasseur).unwrap(),
            None
        );
    }
}
