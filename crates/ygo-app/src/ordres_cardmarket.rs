// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La règle des versions Cardmarket, mesurée sur tout le catalogue.
//!
//! # Pourquoi — 2026-11-02
//!
//! Chez Cardmarket, les versions d'une carte dans un set (« V.1 », « V.2 »…)
//! suivent l'ordre de ses raretés. La règle de l'application
//! ([`ygo_core::cardmarket::ordonner`]) le connaît pour les raretés
//! « normales » (Common, Rare, Super, Ultra, Secret…), qui viennent dans
//! l'ordre croissant ; entre raretés **spéciales** (Ultimate, Collector's,
//! Starlight, Quarter Century, Grand Master…), elle ne sait que quelques
//! paires. Une paire qu'elle ne tranche pas laisse la carte sans lien direct
//! ni prix exact : l'application cherche au lieu de deviner.
//!
//! Cette mesure dit **quelles** paires manquent, et combien de cartes chacune
//! prive de lien, pour écrire les règles qui comptent d'abord. Elle n'émet
//! aucune requête : l'index et `cardinfo.db` suffisent.
//!
//! # Ce que disent les prix
//!
//! Quand une seule paire reste ouverte, ses deux raretés sont voisines dans
//! l'ordre : elles occupent les versions V.k et V.k+1. Si l'on sait dans quel
//! ordre le set a été saisi ([`Sens`]), on connaît les deux produits — et
//! leurs prix. La version la plus chère est, sauf exception, la rareté la plus
//! précieuse : la carte **témoigne** du sens de la paire. Il faut des raretés
//! de valeurs bien distinctes ([`ECART_DE_VALEUR`]) et des prix nettement
//! différents ([`RAPPORT_DE_PRIX`]) ; un témoin n'est pas une preuve, une page
//! Cardmarket l'est.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use ygo_core::cardmarket::{self as regles, Indetermination, Region, Sens};
use ygo_core::paths::Paths;

use crate::cardmarket::{
    self, sens_de_la_carte, valeur_indicative, Base, Cible, Connues, Extension, Index, Prix,
    Raison, ECART_DE_PRIX_SIGNIFICATIF,
};
use crate::error::Result;

/// Écart minimal entre les valeurs a priori des deux raretés pour que leurs
/// prix témoignent.
pub const ECART_DE_VALEUR: f64 = 1.0;

/// Rapport minimal entre les prix des deux versions pour qu'elles
/// témoignent.
pub const RAPPORT_DE_PRIX: f64 = 1.5;

/// Exemples gardés par paire, un par extension.
const EXEMPLES: usize = 3;

/// Extensions détaillées par paire dans le rapport.
const EXTENSIONS_DETAILLEES: usize = 12;

/// Paires détaillées par région dans le rapport.
const PAIRES_DETAILLEES: usize = 15;

/// Extensions listées pour les versions de même rareté.
const DIVERGENCES_LISTEES: usize = 40;

/// Ce que la règle donne, par région.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compte {
    /// Cartes à plusieurs versions que la base connaît.
    pub cartes: usize,
    /// Ordre entièrement trouvé (règle, ou ordre relevé).
    pub ordonnees: usize,
    /// Au moins une paire non tranchée.
    pub indeterminees: usize,
    /// La règle se contredit.
    pub contradictoires: usize,
    /// Cardmarket a plus de versions que la base n'a de raretés : deux
    /// versions de même rareté (illustration alternative, Overframe).
    pub versions_en_plus: usize,
    /// Cardmarket en a moins.
    pub versions_en_moins: usize,
    /// Tirages de la base (carte × rareté) dont l'extension a la carte.
    pub tirages: usize,
    /// … ce que l'application en fait, par raison ([`libelle_de_raison`]).
    pub raisons: BTreeMap<&'static str, usize>,
    /// Tirages encore sans lien faute de sens de saisie, dont les prix seuls
    /// désigneraient chaque version ([`attribution_par_les_prix`]).
    pub par_les_prix: usize,
    /// Contrôle sur les cartes d'un set au sens **connu** : l'attribution par
    /// le sens de la carte rejoint-elle celle du set ? (accords, désaccords)
    pub controle_sens_de_la_carte: (usize, usize),
    /// Même contrôle pour l'attribution par les prix seuls.
    pub controle_par_les_prix: (usize, usize),
}

/// Le nom court d'une raison, pour les comptes.
#[must_use]
pub fn libelle_de_raison(raison: &Raison) -> &'static str {
    match raison {
        Raison::VersionUnique => "direct — version unique",
        Raison::Ordonnee => "direct — ordre et sens de saisie connus",
        Raison::OrdreDeSaisieSuppose => "direct — sens de saisie supposé (TCG)",
        Raison::SensDeLaCarte => "direct — sens de saisie lu dans les prix de la carte",
        Raison::SaisieMelangee => "recherche — set saisi dans le désordre ou sens inconnu",
        Raison::SaisieDouteuse => "recherche — prix contraires à la saisie",
        Raison::OrdreDesVersionsInconnu => "recherche — paire de raretés non tranchée",
        Raison::PlusieursVersionsDeLaRarete => "recherche — versions de même rareté",
        Raison::IllustrationsSaisiesAPart => "recherche — illustrations saisies à part",
        Raison::SeuleFicheEnVente => "direct — seule fiche en vente",
        Raison::NombreDeVersions { .. } => "recherche — nombre de versions différent",
        Raison::PlusieursNumeros => "recherche — plusieurs numéros",
        Raison::RareteInconnue => "recherche — rareté inconnue",
        Raison::CarteAbsente => "recherche — carte absente de l'extension",
        Raison::ExtensionInconnue => "recherche — extension inconnue",
        Raison::NumeroInconnu => "recherche — numéro inconnu",
    }
}

/// Les témoins d'une paire `(a, b)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Votes {
    /// Cartes où `a` passe avant `b`.
    pub a_avant_b: u32,
    /// Cartes où `b` passe avant `a`.
    pub b_avant_a: u32,
}

impl Votes {
    fn compter(&mut self, a_avant_b: bool) {
        if a_avant_b {
            self.a_avant_b += 1;
        } else {
            self.b_avant_a += 1;
        }
    }

    fn total(self) -> u32 {
        self.a_avant_b + self.b_avant_a
    }
}

/// Une paire dans une extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ParExtension {
    /// `idExpansion`.
    pub id: i64,
    /// Nom chez Cardmarket.
    pub libelle: Option<String>,
    /// Date de sortie du set.
    pub sortie: Option<String>,
    /// Sens de saisie.
    pub sens: Sens,
    /// Cartes où la paire reste ouverte.
    pub cartes: usize,
    /// Témoins d'un set au sens de saisie connu.
    pub surs: Votes,
    /// Témoins d'un set TCG au sens supposé (dans l'ordre).
    pub supposes: Votes,
}

/// Une carte à montrer pour vérifier une paire sur sa page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exemple {
    /// `idExpansion`.
    pub extension: i64,
    /// Nom de l'extension.
    pub libelle: Option<String>,
    /// Nom de la carte (normalisé, celui du rapprochement).
    pub carte: String,
    /// Toutes ses versions, dans l'ordre de saisie supposé : (produit, prix
    /// de référence en centimes).
    pub produits: Vec<(i64, Option<u32>)>,
    /// La place de la paire : V.(k+1) et V.(k+2).
    pub k: usize,
    /// Toutes les raretés de la carte.
    pub raretes: Vec<String>,
}

/// Une paire de raretés que la règle ne tranche pas.
#[derive(Debug, Clone, PartialEq)]
pub struct Paire {
    /// Région.
    pub region: Region,
    /// La première rareté, par ordre alphabétique.
    pub a: String,
    /// La seconde.
    pub b: String,
    /// Cartes où elle reste ouverte.
    pub cartes: usize,
    /// … dont celles où c'est la seule paire ouverte.
    pub seule: usize,
    /// Témoins sûrs.
    pub surs: Votes,
    /// Témoins supposés.
    pub supposes: Votes,
    /// Par extension.
    pub extensions: BTreeMap<i64, ParExtension>,
    /// Cartes à vérifier.
    pub exemples: Vec<Exemple>,
}

/// Une extension où Cardmarket a plus de versions que la base de raretés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// `idExpansion`.
    pub id: i64,
    /// Nom.
    pub libelle: Option<String>,
    /// Région.
    pub region: Region,
    /// Sortie.
    pub sortie: Option<String>,
    /// Cartes concernées.
    pub cartes: usize,
    /// Une carte, son nombre de versions et ses raretés selon la base.
    pub exemple: (String, usize, Vec<String>),
}

/// Le résultat de la mesure.
#[derive(Debug, Clone, Default)]
pub struct Mesure {
    /// Extensions examinées.
    pub extensions: usize,
    /// Par région.
    pub comptes: HashMap<Region, Compte>,
    /// Les paires ouvertes, les plus fréquentes d'abord.
    pub paires: Vec<Paire>,
    /// Les versions de même rareté, par extension, les plus nombreuses d'abord.
    pub divergences: Vec<Divergence>,
}

/// Lit l'index (refait s'il le faut) et la base, puis mesure — `None` sans
/// index : fichiers Cardmarket pas encore téléchargés.
///
/// # Errors
///
/// Si l'index ne peut être construit, ou si `cardinfo.db` ne se lit pas.
pub fn mesurer_installation(paths: &Paths) -> Result<Option<Mesure>> {
    if let cardmarket::EtatIndex::Impossible(raison) =
        cardmarket::reconstruire_si_besoin(paths, false)?
    {
        tracing::warn!(raison, "mesure des ordres Cardmarket : pas d'index");
        return Ok(None);
    }
    let Some(index) = cardmarket::lire(paths) else {
        return Ok(None);
    };
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let mut base = Base::lire(&conn)?;
    base.ajouter_classeurs_hors_base(paths);
    Ok(Some(mesurer(&index, &base)))
}

/// La mesure elle-même, sans fichier.
#[must_use]
pub fn mesurer(index: &Index, base: &Base) -> Mesure {
    let mut mesure = Mesure::default();
    let mut paires: HashMap<(Region, String, String), Paire> = HashMap::new();
    let mut divergences: HashMap<i64, Divergence> = HashMap::new();
    for ext in &index.extensions {
        let Some(cartes_locale) = base.cartes_de(&ext.set_uuid, &ext.langue) else {
            continue;
        };
        mesure.extensions += 1;
        let ctx = ext.contexte();
        let compte = mesure.comptes.entry(ext.region).or_default();
        for (nom, versions) in &ext.cartes {
            let n = versions.ids.len();
            let Some(numeros) = cartes_locale.get(nom) else {
                continue;
            };
            let raretes: Vec<String> = numeros
                .values()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            // Ce que l'application fait de chaque tirage, comme pour une ligne
            // de classeur tirée de la base.
            let connues = Connues {
                base: numeros.clone(),
                classeur: Vec::new(),
            };
            for r in &raretes {
                let offre = cardmarket::decider(
                    ext,
                    versions,
                    &connues,
                    r,
                    nom,
                    &index.prix,
                    String::new(),
                );
                debug_assert_eq!(
                    matches!(offre.cible, Cible::Produit(_)),
                    libelle_de_raison(&offre.raison).starts_with("direct")
                );
                compte.tirages += 1;
                *compte
                    .raisons
                    .entry(libelle_de_raison(&offre.raison))
                    .or_default() += 1;
                if offre.raison == Raison::SaisieMelangee {
                    if let Some(ordre) = regles::ordonner(&raretes, &ctx) {
                        if attribution_par_les_prix(&ordre, &versions.ids, &index.prix).is_some() {
                            compte.par_les_prix += 1;
                        }
                    }
                }
            }
            if n < 2 {
                continue;
            }
            // Les attributions par carte, éprouvées là où le sens du set est
            // connu.
            if raretes.len() == n {
                let du_set: Option<Vec<i64>> = match ext.sens {
                    Sens::DansLOrdre => Some(versions.ids.clone()),
                    Sens::Inverse => Some(versions.ids.iter().rev().copied().collect()),
                    Sens::Inconnu | Sens::Melange => None,
                };
                if let (Some(du_set), Some(ordre)) = (du_set, regles::ordonner(&raretes, &ctx)) {
                    if cardmarket::vraisemblable(&ordre, &du_set, &index.prix) {
                        let eprouver = |essai: Option<Vec<i64>>, c: &mut (usize, usize)| {
                            if let Some(essai) = essai {
                                if essai == du_set {
                                    c.0 += 1;
                                } else {
                                    c.1 += 1;
                                }
                            }
                        };
                        eprouver(
                            sens_de_la_carte(&ordre, &versions.ids, &index.prix),
                            &mut compte.controle_sens_de_la_carte,
                        );
                        eprouver(
                            attribution_par_les_prix(&ordre, &versions.ids, &index.prix),
                            &mut compte.controle_par_les_prix,
                        );
                    }
                }
            }
            compte.cartes += 1;
            if raretes.len() != n {
                let releve = regles::ordres_confirmes(ext.id)
                    .iter()
                    .any(|s| s.len() == n && raretes.iter().all(|r| s.contains(r)));
                if releve {
                    compte.ordonnees += 1;
                } else if n > raretes.len() {
                    compte.versions_en_plus += 1;
                    divergences
                        .entry(ext.id)
                        .or_insert_with(|| Divergence {
                            id: ext.id,
                            libelle: ext.libelle.clone(),
                            region: ext.region,
                            sortie: ext.sortie.clone(),
                            cartes: 0,
                            exemple: (nom.clone(), n, raretes.clone()),
                        })
                        .cartes += 1;
                } else {
                    compte.versions_en_moins += 1;
                }
                continue;
            }
            let ouvertes = match regles::paires_indeterminees(&raretes, &ctx) {
                Indetermination::Contradiction => {
                    compte.contradictoires += 1;
                    continue;
                }
                Indetermination::Paires(p) if p.is_empty() => {
                    compte.ordonnees += 1;
                    continue;
                }
                Indetermination::Paires(p) => p,
            };
            compte.indeterminees += 1;
            let seule = ouvertes.len() == 1;
            for (a, b, k) in &ouvertes {
                let paire = paires
                    .entry((ext.region, a.clone(), b.clone()))
                    .or_insert_with(|| Paire {
                        region: ext.region,
                        a: a.clone(),
                        b: b.clone(),
                        cartes: 0,
                        seule: 0,
                        surs: Votes::default(),
                        supposes: Votes::default(),
                        extensions: BTreeMap::new(),
                        exemples: Vec::new(),
                    });
                paire.cartes += 1;
                let par_ext = paire
                    .extensions
                    .entry(ext.id)
                    .or_insert_with(|| ParExtension {
                        id: ext.id,
                        libelle: ext.libelle.clone(),
                        sortie: ext.sortie.clone(),
                        sens: ext.sens,
                        cartes: 0,
                        surs: Votes::default(),
                        supposes: Votes::default(),
                    });
                par_ext.cartes += 1;
                if !seule {
                    continue;
                }
                paire.seule += 1;
                if let Some((a_avant_b, sur)) = temoin(ext, &versions.ids, a, b, *k, &index.prix) {
                    if sur {
                        paire.surs.compter(a_avant_b);
                        par_ext.surs.compter(a_avant_b);
                    } else {
                        paire.supposes.compter(a_avant_b);
                        par_ext.supposes.compter(a_avant_b);
                    }
                }
                if paire.exemples.len() < EXEMPLES
                    && paire.exemples.iter().all(|e| e.extension != ext.id)
                {
                    paire.exemples.push(Exemple {
                        extension: ext.id,
                        libelle: ext.libelle.clone(),
                        carte: nom.clone(),
                        produits: ids_dans_l_ordre(ext, &versions.ids)
                            .iter()
                            .map(|id| (*id, index.prix.get(id).and_then(Prix::reference)))
                            .collect(),
                        k: *k,
                        raretes: raretes.clone(),
                    });
                }
            }
        }
    }
    mesure.paires = paires.into_values().collect();
    mesure.paires.sort_by(|x, y| {
        y.cartes
            .cmp(&x.cartes)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
    });
    mesure.divergences = divergences.into_values().collect();
    mesure
        .divergences
        .sort_by(|x, y| y.cartes.cmp(&x.cartes).then_with(|| x.id.cmp(&y.id)));
    mesure
}

/// L'attribution que les prix seuls désignent : chaque rareté a une valeur,
/// elles sont écartées d'au moins [`ECART_DE_VALEUR`] de proche en proche,
/// chaque produit a un prix, et les prix s'écartent d'au moins
/// [`RAPPORT_DE_PRIX`] de proche en proche — le moins cher est la moins
/// précieuse, et ainsi de suite. Rend les identifiants dans l'ordre des
/// versions.
#[must_use]
pub fn attribution_par_les_prix(
    ordre: &[String],
    ids: &[i64],
    prix: &HashMap<i64, Prix>,
) -> Option<Vec<i64>> {
    if ordre.len() != ids.len() || ordre.len() < 2 {
        return None;
    }
    let mut valeurs: Vec<(f64, usize)> = ordre
        .iter()
        .enumerate()
        .map(|(i, r)| valeur_indicative(r).map(|v| (v, i)))
        .collect::<Option<_>>()?;
    valeurs.sort_by(|a, b| a.0.total_cmp(&b.0));
    if valeurs
        .windows(2)
        .any(|w| matches!(w, [a, b] if b.0 - a.0 < ECART_DE_VALEUR))
    {
        return None;
    }
    let mut produits: Vec<(u32, i64)> = ids
        .iter()
        .map(|id| prix.get(id).and_then(Prix::reference).map(|p| (p, *id)))
        .collect::<Option<_>>()?;
    produits.sort_unstable();
    if produits.windows(2).any(|w| {
        matches!(w, [a, b] if b.0 - a.0 < ECART_DE_PRIX_SIGNIFICATIF
            || f64::from(b.0) < RAPPORT_DE_PRIX * f64::from(a.0))
    }) {
        return None;
    }
    // Le k-ième moins cher va à la k-ième moins précieuse.
    let mut attribues = vec![0_i64; ordre.len()];
    for ((_, place), (_, id)) in valeurs.iter().zip(&produits) {
        *attribues.get_mut(*place)? = *id;
    }
    Some(attribues)
}

/// Les produits dans l'ordre des versions, selon le sens de saisie — tels
/// quels quand il est inconnu.
fn ids_dans_l_ordre(ext: &Extension, ids: &[i64]) -> Vec<i64> {
    if ext.sens == Sens::Inverse {
        ids.iter().rev().copied().collect()
    } else {
        ids.to_vec()
    }
}

/// Ce que les prix d'une carte disent de la paire `(a, b)`, voisine aux
/// places `k` et `k + 1` : `(a passe avant b, témoin sûr)`.
///
/// Sûr quand le sens de saisie du set est connu ; supposé pour un set TCG au
/// sens inconnu, réputé dans l'ordre (70 sets sur 78 testables le sont).
fn temoin(
    ext: &Extension,
    ids: &[i64],
    a: &str,
    b: &str,
    k: usize,
    prix: &HashMap<i64, Prix>,
) -> Option<(bool, bool)> {
    let sur = match ext.sens {
        Sens::DansLOrdre | Sens::Inverse => true,
        Sens::Inconnu if ext.region == Region::Tcg => false,
        Sens::Inconnu | Sens::Melange => return None,
    };
    let ordre = ids_dans_l_ordre(ext, ids);
    let premier = prix.get(ordre.get(k)?)?.reference()?;
    let second = prix.get(ordre.get(k + 1)?)?.reference()?;
    let (va, vb) = (valeur_indicative(a)?, valeur_indicative(b)?);
    if (va - vb).abs() < ECART_DE_VALEUR {
        return None;
    }
    let (bas, haut) = (premier.min(second), premier.max(second));
    if haut - bas < ECART_DE_PRIX_SIGNIFICATIF || f64::from(haut) < RAPPORT_DE_PRIX * f64::from(bas)
    {
        return None;
    }
    // La moins chère des deux places porte la moins précieuse des raretés.
    Some(((premier < second) == (va < vb), sur))
}

// ─────────────────────────────────────────────────────────────────────────────
// Rapport
// ─────────────────────────────────────────────────────────────────────────────

fn nom_de_region(region: Region) -> &'static str {
    match region {
        Region::Tcg => "TCG",
        Region::Ocg => "OCG",
    }
}

fn annee(sortie: Option<&str>) -> Option<&str> {
    sortie.and_then(|s| s.get(..4))
}

fn votes(v: Votes, a: &str, b: &str) -> String {
    if v.total() == 0 {
        "—".to_owned()
    } else {
        format!(
            "{} « {a} » d'abord · {} « {b} » d'abord",
            v.a_avant_b, v.b_avant_a
        )
    }
}

fn prix_court(prix: Option<u32>) -> String {
    prix.map_or_else(|| "sans prix".to_owned(), |c| cardmarket::euros(c.into()))
}

/// Le rapport, en Markdown.
#[must_use]
pub fn rapport(mesure: &Mesure, date: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# Ordre des versions Cardmarket — ce que la règle ne tranche pas\n"
    );
    let _ = writeln!(
        s,
        "Mesure du {date} sur les {} extensions Cardmarket rapprochées d'un set de la base, \
         sans aucune requête. Une carte « à plusieurs versions » a au moins deux produits dans \
         son extension ; l'ordre de ses raretés donne la version de chacune (V.1, V.2…).\n",
        mesure.extensions
    );
    let _ = writeln!(
        s,
        "**Témoins** : une carte dont la paire est la seule ouverte, dans un set au sens de \
         saisie connu (**sûrs**) ou, en TCG, supposé dans l'ordre (**supposés**). La version la \
         plus chère y est réputée la rareté la plus précieuse (valeurs a priori écartées d'au \
         moins {ECART_DE_VALEUR}, prix d'au moins ×{RAPPORT_DE_PRIX}). Un témoin oriente ; la \
         page Cardmarket tranche.\n"
    );

    let _ = writeln!(s, "## Bilan\n");
    let _ = writeln!(
        s,
        "| Région | Cartes à plusieurs versions | Ordre trouvé | Paire non tranchée | \
         Contradiction | Versions de même rareté | Cardmarket en a moins |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|");
    for region in [Region::Tcg, Region::Ocg] {
        let c = mesure.comptes.get(&region).cloned().unwrap_or_default();
        let part = |x: usize| {
            if c.cartes == 0 {
                String::new()
            } else {
                #[allow(clippy::cast_precision_loss)]
                let p = x as f64 * 100.0 / c.cartes as f64;
                format!(" ({p:.1} %)")
            }
        };
        let _ = writeln!(
            s,
            "| {} | {} | {}{} | {}{} | {} | {}{} | {} |",
            nom_de_region(region),
            c.cartes,
            c.ordonnees,
            part(c.ordonnees),
            c.indeterminees,
            part(c.indeterminees),
            c.contradictoires,
            c.versions_en_plus,
            part(c.versions_en_plus),
            c.versions_en_moins
        );
    }

    let _ = writeln!(
        s,
        "\n## Ce que l'application fait des tirages de la base\n\nChaque tirage (carte × rareté) \
         d'une carte que l'extension Cardmarket connaît, comme une ligne de classeur tirée de la \
         base — sans les précisions d'un classeur (illustration 2, Overframe).\n"
    );
    let _ = writeln!(s, "| Issue | TCG | OCG |");
    let _ = writeln!(s, "|---|---|---|");
    let vide = Compte::default();
    let tcg = mesure.comptes.get(&Region::Tcg).unwrap_or(&vide);
    let ocg = mesure.comptes.get(&Region::Ocg).unwrap_or(&vide);
    let issues: BTreeSet<&str> = tcg
        .raisons
        .keys()
        .chain(ocg.raisons.keys())
        .copied()
        .collect();
    let cellule = |c: &Compte, issue: &str| {
        let x = c.raisons.get(issue).copied().unwrap_or(0);
        if c.tirages == 0 {
            x.to_string()
        } else {
            #[allow(clippy::cast_precision_loss)]
            let p = x as f64 * 100.0 / c.tirages as f64;
            format!("{x} ({p:.1} %)")
        }
    };
    for issue in issues {
        let _ = writeln!(
            s,
            "| {issue} | {} | {} |",
            cellule(tcg, issue),
            cellule(ocg, issue)
        );
    }
    let _ = writeln!(s, "| **total** | {} | {} |", tcg.tirages, ocg.tirages);
    let _ = writeln!(
        s,
        "\nLe sens de saisie de **la carte** (ses produits, créés dans l'ordre ou à rebours, \
         collent à ses prix dans un seul des deux sens) supplée celui du set depuis le \
         2026-11-02. Parmi les tirages encore sans lien faute de sens, ceux que les **prix \
         seuls** désigneraient (versions toutes chiffrées, nettement écartées ; non appliqué) : \
         **{} TCG, {} OCG**.",
        tcg.par_les_prix, ocg.par_les_prix
    );
    let _ = writeln!(
        s,
        "\nÉprouvées sur les cartes des sets au sens connu (accords / désaccords avec \
         l'attribution du set) : sens de la carte **{}/{} TCG, {}/{} OCG** ; prix seuls \
         **{}/{} TCG, {}/{} OCG**.",
        tcg.controle_sens_de_la_carte.0,
        tcg.controle_sens_de_la_carte.1,
        ocg.controle_sens_de_la_carte.0,
        ocg.controle_sens_de_la_carte.1,
        tcg.controle_par_les_prix.0,
        tcg.controle_par_les_prix.1,
        ocg.controle_par_les_prix.0,
        ocg.controle_par_les_prix.1
    );

    for region in [Region::Tcg, Region::Ocg] {
        let paires: Vec<&Paire> = mesure
            .paires
            .iter()
            .filter(|p| p.region == region)
            .collect();
        let _ = writeln!(
            s,
            "\n## Paires non tranchées — {} ({})\n",
            nom_de_region(region),
            paires.len()
        );
        if paires.is_empty() {
            let _ = writeln!(s, "Aucune.");
            continue;
        }
        let _ = writeln!(
            s,
            "| Paire | Cartes | dont seule ouverte | Sets | Années | Témoins sûrs | Témoins supposés |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|---|");
        for p in &paires {
            let annees: BTreeSet<&str> = p
                .extensions
                .values()
                .filter_map(|e| annee(e.sortie.as_deref()))
                .collect();
            let plage = match (annees.first(), annees.last()) {
                (Some(x), Some(y)) if x == y => (*x).to_owned(),
                (Some(x), Some(y)) => format!("{x}–{y}"),
                _ => "?".to_owned(),
            };
            let _ = writeln!(
                s,
                "| {} / {} | {} | {} | {} | {plage} | {} | {} |",
                p.a,
                p.b,
                p.cartes,
                p.seule,
                p.extensions.len(),
                votes(p.surs, &p.a, &p.b),
                votes(p.supposes, &p.a, &p.b)
            );
        }
        for p in paires.iter().take(PAIRES_DETAILLEES) {
            let _ = writeln!(
                s,
                "\n### {} — {} / {} ({} cartes)\n",
                nom_de_region(region),
                p.a,
                p.b,
                p.cartes
            );
            let mut exts: Vec<&ParExtension> = p.extensions.values().collect();
            exts.sort_by(|x, y| y.cartes.cmp(&x.cartes).then_with(|| x.id.cmp(&y.id)));
            let _ = writeln!(
                s,
                "| Extension | Sortie | Sens | Cartes | Témoins sûrs | Témoins supposés |"
            );
            let _ = writeln!(s, "|---|---|---|---|---|---|");
            for e in exts.iter().take(EXTENSIONS_DETAILLEES) {
                let _ = writeln!(
                    s,
                    "| {} ({}) | {} | {:?} | {} | {} | {} |",
                    e.libelle.as_deref().unwrap_or("?"),
                    e.id,
                    e.sortie.as_deref().unwrap_or("?"),
                    e.sens,
                    e.cartes,
                    votes(e.surs, &p.a, &p.b),
                    votes(e.supposes, &p.a, &p.b)
                );
            }
            if exts.len() > EXTENSIONS_DETAILLEES {
                let _ = writeln!(
                    s,
                    "\n… et {} autres extensions.",
                    exts.len() - EXTENSIONS_DETAILLEES
                );
            }
            if !p.exemples.is_empty() {
                let _ = writeln!(
                    s,
                    "\nÀ vérifier — la paire occupe les versions en gras ; le titre de la page \
                     dit « V.n - rareté » :\n"
                );
                for x in &p.exemples {
                    let liens: Vec<String> = x
                        .produits
                        .iter()
                        .enumerate()
                        .map(|(i, (id, prix))| {
                            let marque = if i == x.k || i == x.k + 1 { "**" } else { "" };
                            format!(
                                "{marque}V.{} ?{marque} [{id}]({}) {}",
                                i + 1,
                                regles::url_produit("fr", *id),
                                prix_court(*prix)
                            )
                        })
                        .collect();
                    let _ = writeln!(
                        s,
                        "- {} — `{}` ({}) : la paire en V.{}–V.{} — {}",
                        x.libelle.as_deref().unwrap_or("?"),
                        x.carte,
                        x.raretes.join(", "),
                        x.k + 1,
                        x.k + 2,
                        liens.join(" · ")
                    );
                }
            }
        }
    }

    let _ = writeln!(
        s,
        "\n## Versions de même rareté ({} extensions)\n\nCardmarket a plus de versions que la \
         base n'a de raretés : illustrations alternatives, Overframe. Seul le classeur les \
         distingue (fichier d'image), et seul un ordre relevé les place.\n",
        mesure.divergences.len()
    );
    let _ = writeln!(s, "| Extension | Région | Sortie | Cartes | Exemple |");
    let _ = writeln!(s, "|---|---|---|---|---|");
    for d in mesure.divergences.iter().take(DIVERGENCES_LISTEES) {
        let (carte, n, raretes) = &d.exemple;
        let _ = writeln!(
            s,
            "| {} ({}) | {} | {} | {} | `{carte}` : {n} versions, {} |",
            d.libelle.as_deref().unwrap_or("?"),
            d.id,
            nom_de_region(d.region),
            d.sortie.as_deref().unwrap_or("?"),
            d.cartes,
            raretes.join(", ")
        );
    }
    if mesure.divergences.len() > DIVERGENCES_LISTEES {
        let _ = writeln!(
            s,
            "\n… et {} autres extensions.",
            mesure.divergences.len() - DIVERGENCES_LISTEES
        );
    }
    s
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn prix(paires: &[(i64, u32)]) -> HashMap<i64, Prix> {
        paires
            .iter()
            .map(|(id, t)| {
                (
                    *id,
                    Prix {
                        tendance: Some(*t),
                        ..Prix::default()
                    },
                )
            })
            .collect()
    }

    fn ordre(r: &[&str]) -> Vec<String> {
        r.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn les_prix_seuls_designent_chaque_version() {
        let o = ordre(&["Ultra Rare", "Secret Rare", "Starlight Rare"]);
        let p = prix(&[(1, 100), (2, 5000), (3, 300)]);
        assert_eq!(
            attribution_par_les_prix(&o, &[3, 1, 2], &p),
            Some(vec![1, 3, 2])
        );
        // Deux prix trop proches : rien.
        let p = prix(&[(1, 100), (2, 5000), (3, 120)]);
        assert_eq!(attribution_par_les_prix(&o, &[3, 1, 2], &p), None);
        // Un produit sans prix : rien.
        let p = prix(&[(1, 100), (2, 5000)]);
        assert_eq!(attribution_par_les_prix(&o, &[3, 1, 2], &p), None);
        // Deux raretés de valeurs voisines : rien.
        let o = ordre(&["Secret Rare", "Ultimate Rare"]);
        let p = prix(&[(1, 100), (2, 5000)]);
        assert_eq!(attribution_par_les_prix(&o, &[1, 2], &p), None);
    }

    #[test]
    fn le_sens_de_la_carte_suit_ses_prix() {
        let o = ordre(&["Ultra Rare", "Starlight Rare"]);
        // Créée à rebours : la Starlight, la plus chère, a le premier identifiant.
        let p = prix(&[(1, 5000), (2, 100)]);
        assert_eq!(sens_de_la_carte(&o, &[1, 2], &p), Some(vec![2, 1]));
        let p = prix(&[(1, 100), (2, 5000)]);
        assert_eq!(sens_de_la_carte(&o, &[1, 2], &p), Some(vec![1, 2]));
        // Sans prix nettement écartés, pas de sens.
        let p = prix(&[(1, 100), (2, 150)]);
        assert_eq!(sens_de_la_carte(&o, &[1, 2], &p), None);
    }
}
