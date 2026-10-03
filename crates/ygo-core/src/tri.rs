// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Tri et filtrage des cartes d'un classeur.
//!
//! Portage de `module/gestion_rarete/tri_carte.py` (243 lignes). Module
//! **pur** : ni réseau, ni SQLite, ni interface — c'est ce qui permet de le
//! comparer ligne à ligne au Python sur les classeurs réels.
//!
//! # Ce que le tri consulte
//!
//! Cinq champs seulement, quelle que soit la richesse du dictionnaire Python :
//! `name`, `set_code`, `rarity`, `card_image_id`, `extended_art`. D'où le
//! trait [`CarteTriable`] plutôt qu'une structure imposée.
//!
//! # Trois critères, six ordres
//!
//! L'utilisateur réordonne `numero`, `rarete` et `artwork` par glisser-déposer.
//! L'ordre est lu dans `app_config.json` ([`crate::config::Config::ordre_tri`]),
//! et le défaut — `numero → artwork → rarete` — place **artwork avant
//! rarete**. Ce détail est la raison d'être du regroupement des images externes
//! décrit ci-dessous.
//!
//! Après les trois critères vient toujours `(set_code, name)`, départage stable
//! pour les doublons parfaits.
//!
//! # Le piège des images externes
//!
//! Yugipedia héberge **un fichier par rareté** pour une même illustration. Les
//! noms de fichiers diffèrent, donc leurs CRC32 — qui servent d'identifiants —
//! diffèrent aussi. Relevé sur `LOCR-JP001` :
//!
//! | rareté | fichier | id |
//! |---|---|---|
//! | Grand Master Rare | `…-LOCR-JP-OP.png` | `-773089896` |
//! | Prismatic Secret Rare | `…-LOCR-JP-PScR.png` | `-117544476` |
//! | Ultra Rare | `…-LOCR-JP-UR-EA.png` | `-37412837` |
//!
//! Les traiter comme trois artworks distincts leur donnerait trois rangs, dans
//! l'ordre croissant des identifiants — c'est-à-dire dans l'ordre du **hasard
//! des hachages**. Et comme `artwork` est comparé avant `rarete`, la rareté ne
//! serait jamais consultée : la Grand Master Rare s'afficherait en tête parce
//! que son CRC32 est le plus petit.
//!
//! Ces trois lignes sont la même illustration en trois foils. Elles reçoivent
//! donc **un seul rang commun**, placé après les originales, et la rareté
//! reprend la main.
//!
//! # Deux défauts divergents, reproduits
//!
//! Une rareté absente de `rarity_config.json` vaut `9999` au tri (elle finit
//! dernière) et `0` au filtrage (elle devient la moins rare). Ce n'est
//! vraisemblablement pas voulu ; le portage étant iso-fonctionnel, on
//! reproduit. Voir [`crate::rarity`].

use std::collections::BTreeSet;
use std::collections::HashMap;

use crate::config::CritereTri;
use crate::rarity::Priorites;

/// Ce dont le tri a besoin d'une carte de classeur.
///
/// Cinq accesseurs, correspondant aux cinq clés que `tri_carte.py` lit dans le
/// dictionnaire Python. Tout le reste — possession, quantité, URL, `sort_order`
/// — n'entre pas dans l'ordre final.
pub trait CarteTriable {
    /// Nom affiché de la carte, dans la langue courante.
    ///
    /// C'est bien le nom **résolu** : la V1.0.4 lit
    /// `COALESCE(NULLIF(name_fr,''), name)` en français. Il entre à la fois
    /// dans le regroupement des artworks et dans le départage final.
    fn nom(&self) -> &str;

    /// Code de set complet, `RA05-EN134`.
    fn set_code(&self) -> &str;

    /// Libellé de rareté, tel qu'il figure en base.
    fn rarete(&self) -> &str;

    /// Identifiant d'illustration, **déjà normalisé** : `0` quand il est
    /// absent ou nul (`c.get("card_image_id") or 0` en Python).
    ///
    /// Un identifiant **négatif** désigne une image externe (Yugipedia), cf.
    /// [`crate::image_source::est_image_externe`].
    fn card_image_id(&self) -> i64;

    /// La carte est-elle un Overframe (`extended_art = 1`) ?
    fn extended_art(&self) -> bool;
}

/// Carte réduite aux cinq champs du tri.
///
/// Suffit aux tests et aux appelants qui n'ont pas de structure plus riche.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Carte {
    /// Nom résolu.
    pub nom: String,
    /// Code de set complet.
    pub set_code: String,
    /// Libellé de rareté.
    pub rarete: String,
    /// Identifiant d'illustration, `0` si absent.
    pub card_image_id: i64,
    /// Overframe ?
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

impl<T: CarteTriable> CarteTriable for &T {
    fn nom(&self) -> &str {
        (*self).nom()
    }
    fn set_code(&self) -> &str {
        (*self).set_code()
    }
    fn rarete(&self) -> &str {
        (*self).rarete()
    }
    fn card_image_id(&self) -> i64 {
        (*self).card_image_id()
    }
    fn extended_art(&self) -> bool {
        (*self).extended_art()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Critère « numero »
// ─────────────────────────────────────────────────────────────────────────────

/// Clé de tri extraite d'un `set_code` : groupe de lettres, puis numéro.
///
/// Le groupe de lettres sépare les sous-decks d'un set multi-deck. `L26D`
/// mélange `L26D-ENM01..M99`, `L26D-ENS01..S99` et `L26D-ENX01..X99` ; trier
/// sur le numéro seul donnerait `M01, S01, X01, M02, …`. Avec le groupe en
/// tête, on obtient les trois blocs à la suite.
///
/// Le groupe emprunte au `set_code` reçu — aucune allocation.
pub type CleNumero<'a> = (&'a str, i64);

/// Extrait `(groupe de lettres, numéro)` d'un `set_code`.
///
/// Portage de `tri_carte._extract_numero`. Le préfixe de langue Konami — deux
/// lettres, `EN` `FR` `DE` `IT` `JP` `KR` `SP` `PT` `AE` — est retiré avant
/// l'extraction.
///
/// | entrée | sortie |
/// |---|---|
/// | `L26D-ENM01` | `("M", 1)` |
/// | `L26D-ENS03` | `("S", 3)` |
/// | `RA02-EN006` | `("", 6)` |
/// | `LOB-EN001` | `("", 1)` |
/// | `MGED-ENB17` | `("B", 17)` |
/// | `""` | `("", 0)` |
///
/// Tout ce qui ne se laisse pas analyser retombe sur `("", 0)`, comme le
/// `except Exception` du Python.
///
/// # Écart assumé
///
/// Le Python compile `\d+`, qui en mode `str` accepte **tout chiffre
/// Unicode** (catégorie Nd) — un `٠١٢` arabe passerait. Ici seuls les chiffres
/// ASCII sont acceptés. Aucun `set_code` Konami n'en contient d'autres ; la
/// distinction n'est notée que pour ne pas la découvrir plus tard.
pub fn cle_numero(set_code: &str) -> CleNumero<'_> {
    if set_code.is_empty() {
        return ("", 0);
    }
    // `set_code.rsplit("-", 1)[-1]` : ce qui suit le dernier tiret, ou la
    // chaîne entière s'il n'y en a pas.
    let suffixe = match set_code.rfind('-') {
        Some(i) => &set_code[i + 1..],
        None => set_code,
    };

    // `re.match(r"^[A-Z]{2}", suffixe)` — exactement deux majuscules ASCII.
    let deux_majuscules = suffixe
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_uppercase)
        && suffixe
            .as_bytes()
            .get(1)
            .is_some_and(u8::is_ascii_uppercase);
    let apres_langue = if deux_majuscules {
        &suffixe[2..]
    } else {
        suffixe
    };

    // `re.match(r"^([A-Z]*)(\d+)$", apres_langue)` — ancré des deux côtés.
    let octets = apres_langue.as_bytes();
    let coupure = octets
        .iter()
        .position(|b| !b.is_ascii_uppercase())
        .unwrap_or(octets.len());
    let (lettres, chiffres) = apres_langue.split_at(coupure);
    if chiffres.is_empty() || !chiffres.bytes().all(|b| b.is_ascii_digit()) {
        return ("", 0);
    }
    // Python calcule en entiers illimités ; un numéro de plus de 18 chiffres
    // n'existe pas, mais on sature plutôt que de retomber sur ("", 0), qui
    // serait un ordre différent et non un simple débordement.
    let numero = chiffres.parse::<i64>().unwrap_or(i64::MAX);
    (lettres, numero)
}

// ─────────────────────────────────────────────────────────────────────────────
// Rangs d'artwork
// ─────────────────────────────────────────────────────────────────────────────

/// Rang d'artwork de chaque carte : `0` = Art A, `1` = Art B, etc.
///
/// Portage de `tri_carte._compute_art_ranks`, qui renvoie un dictionnaire
/// indexé par `(name, set_code, extended_art, card_image_id)`. Comme ce
/// dictionnaire n'est jamais consulté qu'avec la clé de la carte elle-même, on
/// renvoie directement un rang par carte, dans l'ordre d'entrée.
///
/// Deux règles :
///
/// - l'**Overframe est un artwork distinct** — il forme son propre groupe de
///   rangs, même quand il partage l'identifiant d'image de la version normale
///   (cas OCG courant) ;
/// - les images **externes** (identifiant négatif) partagent **un seul rang**,
///   placé après les originales. Voir la raison en tête de module.
pub fn rangs_artwork<C: CarteTriable>(cartes: &[C]) -> Vec<usize> {
    // Groupe → identifiants d'image rencontrés, triés (BTreeSet reproduit le
    // `sorted()` du Python sans passe supplémentaire).
    let mut groupes: HashMap<(&str, &str, bool), BTreeSet<i64>> = HashMap::new();
    for c in cartes {
        groupes
            .entry((c.nom(), c.set_code(), c.extended_art()))
            .or_default()
            .insert(c.card_image_id());
    }

    cartes
        .iter()
        .map(|c| {
            let ids = match groupes.get(&(c.nom(), c.set_code(), c.extended_art())) {
                Some(ids) => ids,
                // Inatteignable : le groupe vient d'être inséré pour chaque carte.
                None => return 0,
            };
            let img = c.card_image_id();
            if img < 0 {
                // Toutes les externes après toutes les originales, ensemble.
                ids.range(0..).count()
            } else {
                // Rang = nombre d'originales strictement plus petites.
                ids.range(0..img).count()
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tri
// ─────────────────────────────────────────────────────────────────────────────

/// Composantes de tri d'une carte, calculées une fois.
///
/// Le Python recalcule `_extract_numero` à chaque comparaison ; ici la clé est
/// décorée en amont, ce qui ramène le coût de O(n·log n) extractions à n.
struct Cle<'a> {
    numero: CleNumero<'a>,
    rarete: i64,
    artwork: (bool, usize),
    set_code: &'a str,
    nom: &'a str,
}

fn cles<'a, C: CarteTriable>(
    cartes: &'a [C],
    priorites: &Priorites,
    rangs: &[usize],
) -> Vec<Cle<'a>> {
    cartes
        .iter()
        .zip(rangs)
        .map(|(c, &rang)| Cle {
            numero: cle_numero(c.set_code()),
            rarete: priorites.pour_tri(c.rarete()),
            artwork: (c.extended_art(), rang),
            set_code: c.set_code(),
            nom: c.nom(),
        })
        .collect()
}

/// Permutation d'indices produite par le tri.
///
/// Le résultat `p` se lit : « la carte affichée en position `i` est
/// `cartes[p[i]]` ». Renvoyer les indices plutôt que les cartes évite d'imposer
/// `Clone` et rend la comparaison à l'oracle Python immédiate.
///
/// Portage de `tri_carte.sort_cartes`. Le tri est **stable**, comme le
/// `sorted()` de Python : deux cartes de clé identique gardent leur ordre
/// d'entrée.
pub fn ordre_de_tri<C: CarteTriable>(
    cartes: &[C],
    ordre: [CritereTri; 3],
    priorites: &Priorites,
) -> Vec<usize> {
    if cartes.is_empty() {
        return Vec::new();
    }
    let rangs = rangs_artwork(cartes);
    let cles = cles(cartes, priorites, &rangs);

    // Décoration : la clé voyage avec son indice d'origine. Trier ce couple
    // plutôt qu'un vecteur d'indices évite d'indexer un tableau annexe depuis
    // le comparateur — et `sort_by` étant stable, les ex æquo conservent leur
    // ordre d'entrée, comme le `sorted()` de Python.
    let mut decore: Vec<(Cle<'_>, usize)> = cles.into_iter().zip(0..).collect();
    decore.sort_by(|(ka, _), (kb, _)| {
        for critere in ordre {
            let c = match critere {
                CritereTri::Numero => ka.numero.cmp(&kb.numero),
                CritereTri::Rarete => ka.rarete.cmp(&kb.rarete),
                CritereTri::Artwork => ka.artwork.cmp(&kb.artwork),
            };
            if c != std::cmp::Ordering::Equal {
                return c;
            }
        }
        // Départage final, toujours présent : (set_code, name).
        ka.set_code
            .cmp(kb.set_code)
            .then_with(|| ka.nom.cmp(kb.nom))
    });
    decore.into_iter().map(|(_, i)| i).collect()
}

/// Trie les cartes et renvoie la liste réordonnée.
///
/// Confort au-dessus d'[`ordre_de_tri`], pour l'appelant qui possède ses
/// cartes.
pub fn trier<C: CarteTriable>(
    cartes: Vec<C>,
    ordre: [CritereTri; 3],
    priorites: &Priorites,
) -> Vec<C> {
    let permutation = ordre_de_tri(&cartes, ordre, priorites);
    appliquer(cartes, &permutation)
}

/// Réordonne `cartes` selon une permutation d'indices, sans cloner.
fn appliquer<C>(cartes: Vec<C>, permutation: &[usize]) -> Vec<C> {
    let mut cases: Vec<Option<C>> = cartes.into_iter().map(Some).collect();
    permutation
        .iter()
        .filter_map(|&i| cases.get_mut(i).and_then(Option::take))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Filtre « N raretés par artwork »
// ─────────────────────────────────────────────────────────────────────────────

/// Groupe de filtrage : `(set_code, extended_art, rang d'artwork)` vers les
/// couples `(priorité, indice d'entrée)` qui s'y disputent les N places.
type Groupes<'a> = HashMap<(&'a str, bool, usize), Vec<(i64, usize)>>;

/// Indices des cartes conservées par le filtre « N raretés par artwork ».
///
/// Portage de `tri_carte.filtrer_n_raretes_par_artwork`. Dans chaque groupe
/// `(set_code, extended_art, rang d'artwork)`, seules les `n` raretés les plus
/// rares sont gardées. `RA02-EN001` existe en sept raretés ; avec `n = 3` il
/// n'en reste que les trois plus rares.
///
/// `n = 0` ne filtre rien.
///
/// Le résultat est **croissant** : la liste d'entrée est reparcourue dans
/// l'ordre, exactement comme la compréhension finale du Python. Les égalités de
/// priorité se départagent au premier arrivé.
///
/// # Attention
///
/// La priorité employée ici est [`Priorites::pour_filtre`], dont le défaut est
/// `0` — et non `9999` comme au tri. Deux raretés inconnues dans un même groupe
/// se départagent donc à l'ordre d'apparition, et une rareté inconnue est
/// éliminée avant n'importe quelle rareté connue. Divergence du Python,
/// reproduite telle quelle.
pub fn indices_n_raretes_par_artwork<C: CarteTriable>(
    cartes: &[C],
    n: usize,
    priorites: &Priorites,
) -> Vec<usize> {
    if cartes.is_empty() || n == 0 {
        return (0..cartes.len()).collect();
    }
    let rangs = rangs_artwork(cartes);

    // (set_code, extended_art, rang) → (priorité, indice d'entrée).
    // `extended_art` est dans la clé : l'Overframe garde ses N raretés à lui,
    // sans quoi les deux cadres se disputeraient les mêmes N places et la
    // grille se décalerait.
    let mut groupes: Groupes<'_> = HashMap::new();
    for ((i, c), &rang) in cartes.iter().enumerate().zip(&rangs) {
        groupes
            .entry((c.set_code(), c.extended_art(), rang))
            .or_default()
            .push((priorites.pour_filtre(c.rarete()), i));
    }

    let mut gagnants: Vec<usize> = Vec::with_capacity(cartes.len());
    for items in groupes.values_mut() {
        // Plus rare d'abord ; à égalité, premier vu gagnant.
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        gagnants.extend(items.iter().take(n).map(|&(_, i)| i));
    }
    // La compréhension finale du Python reparcourt la liste d'entrée : le
    // résultat est donc croissant, quelle que soit la rareté gagnante.
    gagnants.sort_unstable();
    gagnants
}

/// Applique le filtre et renvoie les cartes conservées, dans l'ordre d'entrée.
pub fn filtrer_n_raretes_par_artwork<C: CarteTriable>(
    cartes: Vec<C>,
    n: usize,
    priorites: &Priorites,
) -> Vec<C> {
    let gardes = indices_n_raretes_par_artwork(&cartes, n, priorites);
    appliquer(cartes, &gardes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::config::ORDRE_TRI_DEFAUT;

    fn carte(nom: &str, code: &str, rarete: &str, img: i64, ext: bool) -> Carte {
        Carte {
            nom: nom.to_owned(),
            set_code: code.to_owned(),
            rarete: rarete.to_owned(),
            card_image_id: img,
            extended_art: ext,
        }
    }

    fn priorites() -> Priorites {
        Priorites::depuis_paires([
            ("Common", 1),
            ("Rare", 2),
            ("Super Rare", 3),
            ("Ultra Rare", 4),
            ("Secret Rare", 5),
            ("Prismatic Secret Rare", 10),
            ("Grand Master Rare", 43),
        ])
    }

    #[test]
    fn numero_exemples_de_la_docstring_python() {
        assert_eq!(cle_numero("L26D-ENM01"), ("M", 1));
        assert_eq!(cle_numero("L26D-ENS03"), ("S", 3));
        assert_eq!(cle_numero("RA02-EN006"), ("", 6));
        assert_eq!(cle_numero("LOB-EN001"), ("", 1));
        assert_eq!(cle_numero("SS01-ENA01"), ("A", 1));
        assert_eq!(cle_numero("MGED-ENB17"), ("B", 17));
        assert_eq!(cle_numero(""), ("", 0));
    }

    #[test]
    fn numero_cas_degrades() {
        // Pas de tiret : le code entier fait office de suffixe.
        assert_eq!(cle_numero("EN001"), ("", 1));
        assert_eq!(cle_numero("001"), ("", 1));
        // Deux lettres seules : plus de chiffres après le préfixe de langue.
        assert_eq!(cle_numero("SET-EN"), ("", 0));
        // Rien d'analysable.
        assert_eq!(cle_numero("SET-ABC"), ("", 0));
        assert_eq!(cle_numero("SET-EN00X"), ("", 0));
        // Minuscules : `[A-Z]` du Python ne les reconnaît pas.
        assert_eq!(cle_numero("set-en001"), ("", 0));
        // Le préfixe de langue n'est retiré que s'il fait deux majuscules :
        // « E001 » n'en a qu'une, rien n'est retiré, et le « E » devient le
        // groupe de lettres. Vérifié contre le Python.
        assert_eq!(cle_numero("SET-E001"), ("E", 1));
        // Seul le dernier tiret compte.
        assert_eq!(cle_numero("A-B-EN012"), ("", 12));
    }

    #[test]
    fn numero_groupe_les_sous_decks() {
        // Sans groupe de lettres, L26D s'entrelacerait : M01, S01, X01, M02…
        let mut codes = ["L26D-ENX01", "L26D-ENM02", "L26D-ENS01", "L26D-ENM01"];
        codes.sort_by_key(|c| cle_numero(c));
        assert_eq!(
            codes,
            ["L26D-ENM01", "L26D-ENM02", "L26D-ENS01", "L26D-ENX01"]
        );
    }

    #[test]
    fn rangs_originaux_croissants_puis_externes_ensemble() {
        // Trois originales et trois externes de la même illustration.
        let cartes = vec![
            carte("Dragon", "LOCR-JP001", "Common", 300, false),
            carte("Dragon", "LOCR-JP001", "Rare", 100, false),
            carte("Dragon", "LOCR-JP001", "Super Rare", 200, false),
            carte(
                "Dragon",
                "LOCR-JP001",
                "Grand Master Rare",
                -773_089_896,
                false,
            ),
            carte(
                "Dragon",
                "LOCR-JP001",
                "Prismatic Secret Rare",
                -117_544_476,
                false,
            ),
            carte("Dragon", "LOCR-JP001", "Ultra Rare", -37_412_837, false),
        ];
        // 100 → 0, 200 → 1, 300 → 2 ; les trois externes partagent le rang 3.
        assert_eq!(rangs_artwork(&cartes), [2, 0, 1, 3, 3, 3]);
    }

    #[test]
    fn overframe_est_un_groupe_de_rangs_separe() {
        // Même nom, même code, même identifiant d'image : seul le cadre change.
        let cartes = vec![
            carte("Dragon", "LOCR-JP001", "Common", 100, false),
            carte("Dragon", "LOCR-JP001", "Common", 100, true),
        ];
        assert_eq!(rangs_artwork(&cartes), [0, 0]);
    }

    #[test]
    fn externes_regroupees_la_rarete_reprend_la_main() {
        // Le cas qui a motivé le regroupement : par identifiant croissant, la
        // Grand Master Rare (-773 089 896, la plus petite) passerait devant.
        let cartes = vec![
            carte(
                "Dragon",
                "LOCR-JP001",
                "Grand Master Rare",
                -773_089_896,
                true,
            ),
            carte(
                "Dragon",
                "LOCR-JP001",
                "Prismatic Secret Rare",
                -117_544_476,
                true,
            ),
            carte("Dragon", "LOCR-JP001", "Ultra Rare", -37_412_837, true),
        ];
        // Ordre par défaut : numero → artwork → rarete. Les trois ont le même
        // numéro et le même rang d'artwork, donc la rareté départage, du plus
        // commun au plus rare : Ultra (4), Prismatic (10), Grand Master (43).
        let ordre = ordre_de_tri(&cartes, ORDRE_TRI_DEFAUT, &priorites());
        assert_eq!(ordre, [2, 1, 0]);
    }

    #[test]
    fn ordre_des_criteres_change_le_resultat() {
        let cartes = vec![
            carte("B", "SET-EN002", "Common", 10, false),
            carte("A", "SET-EN001", "Secret Rare", 20, false),
        ];
        let p = priorites();
        // numero d'abord : 001 avant 002.
        assert_eq!(
            ordre_de_tri(
                &cartes,
                [CritereTri::Numero, CritereTri::Artwork, CritereTri::Rarete],
                &p
            ),
            [1, 0]
        );
        // rarete d'abord : Common (1) avant Secret Rare (5).
        assert_eq!(
            ordre_de_tri(
                &cartes,
                [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork],
                &p
            ),
            [0, 1]
        );
    }

    #[test]
    fn rarete_inconnue_finit_derniere_au_tri() {
        let cartes = vec![
            carte("A", "SET-EN001", "PLatinum Secret Rare", 1, false),
            carte("A", "SET-EN001", "Common", 2, false),
        ];
        // Même numéro ; « PLatinum Secret Rare » (majuscule fautive, 80 lignes
        // en base réelle) vaut 9999, donc passe après.
        let ordre = ordre_de_tri(
            &cartes,
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork],
            &priorites(),
        );
        assert_eq!(ordre, [1, 0]);
    }

    #[test]
    fn tri_stable_sur_doublons_parfaits() {
        let cartes = vec![
            carte("A", "SET-EN001", "Common", 1, false),
            carte("A", "SET-EN001", "Common", 1, false),
            carte("A", "SET-EN001", "Common", 1, false),
        ];
        assert_eq!(
            ordre_de_tri(&cartes, ORDRE_TRI_DEFAUT, &priorites()),
            [0, 1, 2]
        );
    }

    #[test]
    fn filtre_garde_les_n_plus_rares_par_groupe() {
        let cartes = vec![
            carte("A", "RA02-EN001", "Common", 1, false),
            carte("A", "RA02-EN001", "Secret Rare", 1, false),
            carte("A", "RA02-EN001", "Ultra Rare", 1, false),
            carte("A", "RA02-EN001", "Rare", 1, false),
        ];
        let p = priorites();
        // n = 1 : Secret Rare (5).
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 1, &p), [1]);
        // n = 2 : Secret Rare (5) puis Ultra Rare (4), rendues dans l'ordre
        // d'entrée.
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 2, &p), [1, 2]);
        // n = 0 : aucun filtre.
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 0, &p), [0, 1, 2, 3]);
        // n plus grand que le groupe : tout passe.
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 9, &p), [0, 1, 2, 3]);
    }

    #[test]
    fn filtre_separe_les_cadres() {
        let cartes = vec![
            carte("A", "RA02-EN001", "Common", 1, false),
            carte("A", "RA02-EN001", "Secret Rare", 1, false),
            carte("A", "RA02-EN001", "Common", 1, true),
            carte("A", "RA02-EN001", "Secret Rare", 1, true),
        ];
        // Une gagnante pour le cadre normal, une pour l'Overframe.
        assert_eq!(
            indices_n_raretes_par_artwork(&cartes, 1, &priorites()),
            [1, 3]
        );
    }

    #[test]
    fn filtre_rarete_inconnue_vaut_zero_et_non_neuf_mille() {
        let cartes = vec![
            carte("A", "RA02-EN001", "PLatinum Secret Rare", 1, false),
            carte("A", "RA02-EN001", "Common", 1, false),
        ];
        // Au tri l'inconnue vaut 9999 (dernière) ; ici elle vaut 0, donc la
        // Common (1) l'emporte. C'est bien la divergence du Python.
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 1, &priorites()), [1]);
    }

    #[test]
    fn filtre_egalite_premier_vu_gagnant() {
        let cartes = vec![
            carte("A", "RA02-EN001", "Inconnue X", 1, false),
            carte("A", "RA02-EN001", "Inconnue Y", 1, false),
        ];
        assert_eq!(indices_n_raretes_par_artwork(&cartes, 1, &priorites()), [0]);
    }

    #[test]
    fn liste_vide() {
        let cartes: Vec<Carte> = Vec::new();
        assert!(ordre_de_tri(&cartes, ORDRE_TRI_DEFAUT, &priorites()).is_empty());
        assert!(indices_n_raretes_par_artwork(&cartes, 3, &priorites()).is_empty());
    }

    #[test]
    fn trier_et_filtrer_renvoient_les_cartes() {
        let cartes = vec![
            carte("B", "SET-EN002", "Common", 10, false),
            carte("A", "SET-EN001", "Common", 20, false),
        ];
        let triees = trier(cartes.clone(), ORDRE_TRI_DEFAUT, &priorites());
        assert_eq!(triees[0].set_code, "SET-EN001");
        let filtrees = filtrer_n_raretes_par_artwork(cartes, 1, &priorites());
        assert_eq!(filtrees.len(), 2); // deux groupes distincts, une gagnante chacun
    }
}
