// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'inventaire : toutes les cartes possédées, tous classeurs confondus.
//!
//! Portage de `module/inventaire/inventaire_service.py`. C'est la vue
//! transverse — le classeur montre un set à la fois, l'inventaire montre
//! ce qu'on a.
//!
//! # Une ligne est un `rowid`, pas un nom
//!
//! Une même carte peut exister en plusieurs lignes dans un classeur :
//! artworks alternatifs, raretés multiples d'un même tirage. `(nom,
//! set_code, rareté)` ne suffit donc pas à désigner une ligne, et
//! `(classeur, rowid)` est l'identité employée partout ailleurs dans le
//! projet — c'est elle qui garantit que modifier une quantité depuis
//! l'inventaire touche exactement la ligne que le classeur montre.
//!
//! # La variante, et pourquoi elle n'est pas un simple numéro
//!
//! Deux lignes d'un même tirage et d'une même rareté se distinguent par
//! leur illustration. Le Python numérote ces variantes de 1 à *n* et rend
//! 0 quand il n'y en a qu'une — le libellé (« Art 2 », « Overframe »)
//! restant à la charge de l'interface. On garde cette séparation : ici,
//! un rang ; ailleurs, des mots.

use std::collections::BTreeMap;

use ygo_core::paths::Paths;

use crate::error::{AppError, Result};

/// Taille d'un playset — trois exemplaires, la limite de jeu.
pub const PLAYSET: i64 = 3;

/// Une carte possédée, vue depuis l'inventaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carte {
    /// Le classeur d'où elle vient.
    pub classeur: String,
    /// Identité de la ligne dans ce classeur.
    pub rowid: i64,
    /// Nom anglais.
    pub nom: String,
    /// Nom français, vide s'il est inconnu.
    pub nom_fr: String,
    /// Nom du set.
    pub set_nom: String,
    /// Code du tirage.
    pub set_code: String,
    /// Libellé de rareté.
    pub rarete: String,
    /// Nombre d'exemplaires — au moins 1, une carte possédée l'étant.
    pub quantite: i64,
    /// État (`NM`, `M`…), vide s'il n'a jamais été renseigné.
    pub qualite: String,
    /// Illustration en cadre étendu.
    pub overframe: bool,
    /// Identifiant d'image, qui distingue les variantes.
    pub card_image_id: Option<i64>,
    /// Nombre de variantes de ce tirage dans ce classeur (≥ 1).
    pub variantes: u32,
    /// Rang de cette variante, de 1 à `variantes` — **0** si le tirage
    /// n'en a qu'une, pour que l'interface n'ait rien à afficher.
    pub variante: u32,
}

impl Carte {
    /// Le nom à montrer.
    #[must_use]
    pub fn nom_affiche(&self, francais: bool) -> &str {
        if francais && !self.nom_fr.is_empty() {
            &self.nom_fr
        } else {
            &self.nom
        }
    }

    /// A-t-on le playset ?
    #[must_use]
    pub fn playset(&self) -> bool {
        self.quantite >= PLAYSET
    }

    /// Combien d'exemplaires au-delà du playset — jamais négatif.
    #[must_use]
    pub fn surplus(&self) -> i64 {
        (self.quantite - PLAYSET).max(0)
    }
}

/// Renseigne `variantes` et `variante` sur une liste de cartes.
///
/// Une variante est identifiée par `(overframe, card_image_id)`, dans un
/// ordre déterministe : cadre normal avant Overframe, puis par identifiant
/// croissant. Le groupe est `(classeur, set_code, rareté)` — **la rareté en
/// fait partie**, sans quoi les sept tirages d'une carte de RA02
/// compteraient comme sept variantes d'une même illustration.
pub fn numeroter(cartes: &mut [Carte]) {
    let mut signatures: BTreeMap<(String, String, String), Vec<(bool, i64)>> = BTreeMap::new();
    for c in cartes.iter() {
        let groupe = (c.classeur.clone(), c.set_code.clone(), c.rarete.clone());
        let signature = (c.overframe, c.card_image_id.unwrap_or(i64::MIN));
        let e = signatures.entry(groupe).or_default();
        if !e.contains(&signature) {
            e.push(signature);
        }
    }
    for v in signatures.values_mut() {
        v.sort_unstable();
    }
    for c in cartes.iter_mut() {
        let groupe = (c.classeur.clone(), c.set_code.clone(), c.rarete.clone());
        let signature = (c.overframe, c.card_image_id.unwrap_or(i64::MIN));
        let Some(v) = signatures.get(&groupe) else {
            continue;
        };
        c.variantes = u32::try_from(v.len()).unwrap_or(1);
        c.variante = if v.len() > 1 {
            v.iter()
                .position(|s| *s == signature)
                .map_or(0, |i| u32::try_from(i + 1).unwrap_or(0))
        } else {
            0
        };
    }
}

/// Ce sur quoi l'inventaire se filtre.
///
/// Un critère vide ne filtre pas. `nom` et `code` cherchent une
/// sous-chaîne sans tenir compte de la casse ; les autres exigent l'égalité,
/// parce qu'ils viennent d'une liste déroulante remplie avec les valeurs
/// réellement présentes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtre {
    /// Sous-chaîne du nom.
    pub nom: String,
    /// Sous-chaîne du code.
    pub code: String,
    /// Rareté exacte.
    pub rarete: String,
    /// Nom de set exact.
    pub set_nom: String,
    /// Classeur exact.
    pub classeur: String,
    /// État exact ; [`Filtre::SANS_QUALITE`] ne retient que les états vides.
    pub qualite: String,
    /// Ne garder que ce qui n'atteint pas le playset.
    pub sous_playset: bool,
}

impl Filtre {
    /// La valeur de `qualite` qui ne retient que les états non renseignés.
    ///
    /// Un état vide ne peut pas se demander avec la chaîne vide, qui veut
    /// dire « ne pas filtrer ». Il lui faut donc un nom à lui — c'est le
    /// même besoin que le `__VIDE__` du Python.
    pub const SANS_QUALITE: &'static str = "__vide__";

    /// Y a-t-il quelque chose à filtrer ?
    #[must_use]
    pub fn actif(&self) -> bool {
        !self.nom.trim().is_empty()
            || !self.code.trim().is_empty()
            || !self.rarete.is_empty()
            || !self.set_nom.is_empty()
            || !self.classeur.is_empty()
            || !self.qualite.is_empty()
            || self.sous_playset
    }
}

/// Applique un filtre. Fonction pure — c'est elle qui est testée.
#[must_use]
pub fn filtrer<'a>(cartes: &'a [Carte], filtre: &Filtre, francais: bool) -> Vec<&'a Carte> {
    let nom = filtre.nom.trim().to_lowercase();
    let code = filtre.code.trim().to_lowercase();
    cartes
        .iter()
        .filter(|c| {
            if !nom.is_empty() && !c.nom_affiche(francais).to_lowercase().contains(&nom) {
                return false;
            }
            if !code.is_empty() && !c.set_code.to_lowercase().contains(&code) {
                return false;
            }
            if !filtre.rarete.is_empty() && c.rarete != filtre.rarete {
                return false;
            }
            if !filtre.set_nom.is_empty() && c.set_nom != filtre.set_nom {
                return false;
            }
            if !filtre.classeur.is_empty() && c.classeur != filtre.classeur {
                return false;
            }
            if !filtre.qualite.is_empty() {
                if filtre.qualite == Filtre::SANS_QUALITE {
                    if !c.qualite.is_empty() {
                        return false;
                    }
                } else if c.qualite != filtre.qualite {
                    return false;
                }
            }
            if filtre.sous_playset && c.playset() {
                return false;
            }
            true
        })
        .collect()
}

/// Sur quelle colonne l'inventaire se trie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colonne {
    /// Par nom affiché.
    #[default]
    Nom,
    /// Par code de tirage.
    Code,
    /// Par rareté.
    Rarete,
    /// Par nombre d'exemplaires.
    Quantite,
    /// Par surplus au-delà du playset.
    Surplus,
    /// Par état.
    Qualite,
    /// Par classeur.
    Classeur,
}

impl Colonne {
    /// L'en-tête de la colonne.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Nom => "Carte",
            Self::Code => "Code",
            Self::Rarete => "Rareté",
            Self::Quantite => "Quantité",
            Self::Surplus => "Surplus",
            Self::Qualite => "État",
            Self::Classeur => "Classeur",
        }
    }

    /// Toutes les colonnes triables, dans l'ordre d'affichage.
    #[must_use]
    pub fn toutes() -> [Self; 7] {
        [
            Self::Nom,
            Self::Code,
            Self::Rarete,
            Self::Quantite,
            Self::Surplus,
            Self::Qualite,
            Self::Classeur,
        ]
    }
}

/// Trie l'inventaire.
///
/// Le tri est **total** : à valeur égale sur la colonne demandée, il départage
/// par `(classeur, rowid)`. Sans quoi deux affichages successifs de la même
/// liste pourraient présenter deux ordres différents, et une ligne
/// sélectionnée changerait de place sous le curseur.
pub fn trier(cartes: &mut [Carte], colonne: Colonne, descendant: bool, francais: bool) {
    cartes.sort_by(|a, b| {
        let ordre = match colonne {
            Colonne::Nom => a
                .nom_affiche(francais)
                .to_lowercase()
                .cmp(&b.nom_affiche(francais).to_lowercase()),
            Colonne::Code => a.set_code.cmp(&b.set_code),
            Colonne::Rarete => a.rarete.cmp(&b.rarete),
            Colonne::Quantite => a.quantite.cmp(&b.quantite),
            Colonne::Surplus => a.surplus().cmp(&b.surplus()),
            Colonne::Qualite => a.qualite.cmp(&b.qualite),
            Colonne::Classeur => a.classeur.cmp(&b.classeur),
        };
        let ordre = if descendant { ordre.reverse() } else { ordre };
        ordre.then_with(|| (&a.classeur, a.rowid).cmp(&(&b.classeur, b.rowid)))
    });
}

/// Les totaux d'un inventaire : lignes, exemplaires, playsets complets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totaux {
    /// Nombre de lignes.
    pub lignes: usize,
    /// Somme des quantités.
    pub exemplaires: i64,
    /// Lignes ayant au moins un playset.
    pub playsets: usize,
    /// Exemplaires au-delà des playsets.
    pub surplus: i64,
}

/// Additionne ce qu'il y a à additionner.
#[must_use]
pub fn totaux(cartes: &[&Carte]) -> Totaux {
    Totaux {
        lignes: cartes.len(),
        exemplaires: cartes.iter().map(|c| c.quantite).sum(),
        playsets: cartes.iter().filter(|c| c.playset()).count(),
        surplus: cartes.iter().map(|c| c.surplus()).sum(),
    }
}

/// Les valeurs distinctes d'un champ, pour remplir une liste déroulante.
///
/// Triées et sans doublon : ce sont les seules valeurs que le filtre par
/// égalité peut retenir, donc les seules qu'il faut proposer.
#[must_use]
pub fn valeurs(cartes: &[Carte], champ: fn(&Carte) -> &str) -> Vec<String> {
    let mut v: Vec<String> = cartes
        .iter()
        .map(|c| champ(c).to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Une ligne désignée : son classeur, et son `rowid` dans ce classeur.
///
/// C'est l'adresse complète d'une ligne — le `rowid` seul ne suffit pas,
/// chaque base ayant sa propre numérotation.
pub type Adresse = (String, i64);

/// Ce qu'une écriture en masse a changé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touchees {
    /// Nombre de lignes modifiées.
    pub lignes: usize,
    /// Classeurs touchés, dans l'ordre alphabétique.
    pub classeurs: Vec<String>,
}

/// Renseigne l'état d'un lot de cartes, en une transaction par classeur.
///
/// # Pourquoi en masse et pas une par une
///
/// Sur l'installation réelle, 176 cartes de `RA05` et 154 de `LOCR-JP`
/// n'ont aucun état : elles ont été saisies au clic dans le classeur, sans
/// jamais passer par un import. Les renseigner une par une, c'est trois
/// cent trente boîtes de dialogue. L'export les laisse vides à dessein —
/// il n'invente pas un état de conservation — mais il faut bien pouvoir le
/// renseigner autrement qu'à l'unité.
///
/// Une chaîne vide **efface** l'état : c'est la façon de revenir en
/// arrière après s'être trompé de lot.
///
/// # Errors
///
/// Rend une erreur si un classeur ne peut pas être ouvert ou écrit. Les
/// classeurs déjà traités gardent leurs écritures.
pub fn definir_qualite(paths: &Paths, cibles: &[Adresse], qualite: &str) -> Result<Touchees> {
    ecrire(paths, cibles, "qualite", &qualite.trim().to_owned())
}

/// Renseigne l'édition d'un lot de cartes.
///
/// L'édition est ce qui décide **dans quelle colonne** l'export écrit
/// l'état. Une chaîne vide l'efface, et l'export se replie alors sur
/// « 1st Edition ».
///
/// # Errors
///
/// Voir [`definir_qualite`].
pub fn definir_edition(paths: &Paths, cibles: &[Adresse], edition: &str) -> Result<Touchees> {
    ecrire(paths, cibles, "edition", &edition.trim().to_owned())
}

/// Fixe la quantité d'un lot de cartes.
///
/// Une quantité nulle **retire** les cartes de l'inventaire : `possessed`
/// retombe à zéro, comme partout ailleurs dans le projet.
///
/// # Errors
///
/// Voir [`definir_qualite`].
pub fn definir_quantite(paths: &Paths, cibles: &[Adresse], quantite: i64) -> Result<Touchees> {
    let quantite = quantite.max(0);
    appliquer(paths, cibles, |transaction, rowid| {
        transaction.execute(
            "UPDATE cards SET quantite = ?1, possessed = ?2 WHERE rowid = ?3",
            rusqlite::params![quantite, i64::from(quantite > 0), rowid],
        )
    })
}

/// Retire un lot de cartes de l'inventaire, sans effacer leurs lignes.
///
/// La ligne reste dans le classeur — elle décrit un tirage qui existe, que
/// l'utilisateur le possède ou non. Seule la possession retombe.
///
/// # Errors
///
/// Voir [`definir_qualite`].
pub fn retirer(paths: &Paths, cibles: &[Adresse]) -> Result<Touchees> {
    definir_quantite(paths, cibles, 0)
}

/// Écrit une colonne textuelle sur un lot de lignes.
fn ecrire(paths: &Paths, cibles: &[Adresse], colonne: &str, valeur: &String) -> Result<Touchees> {
    // La colonne vient d'ici, jamais de l'appelant : les deux seules
    // valeurs possibles sont écrites en toutes lettres ci-dessous, et la
    // requête n'est donc pas construite à partir d'une donnée.
    let requete = match colonne {
        "qualite" => "UPDATE cards SET qualite = ?1 WHERE rowid = ?2",
        "edition" => "UPDATE cards SET edition = ?1 WHERE rowid = ?2",
        autre => return Err(AppError::Creation(format!("colonne inattendue : {autre}"))),
    };
    appliquer(paths, cibles, |transaction, rowid| {
        transaction.execute(requete, rusqlite::params![valeur, rowid])
    })
}

/// Le squelette commun : grouper par classeur, une transaction chacun.
fn appliquer<F>(paths: &Paths, cibles: &[Adresse], mut ecriture: F) -> Result<Touchees>
where
    F: FnMut(&rusqlite::Transaction<'_>, i64) -> rusqlite::Result<usize>,
{
    let mut par_classeur: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (classeur, rowid) in cibles {
        par_classeur
            .entry(classeur.as_str())
            .or_default()
            .push(*rowid);
    }
    let mut touchees = Touchees::default();
    for (classeur, rowids) in par_classeur {
        let mut conn = ygo_db::connexion::ouvrir(paths.classeur_db(classeur))
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        let transaction = conn
            .transaction()
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        for rowid in &rowids {
            ecriture(&transaction, *rowid)
                .map_err(|e| AppError::Creation(format!("{classeur} rowid {rowid} : {e}")))?;
        }
        transaction
            .commit()
            .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
        touchees.lignes += rowids.len();
        touchees.classeurs.push(classeur.to_owned());
    }
    Ok(touchees)
}

/// Lit l'inventaire d'une installation entière.
///
/// Un classeur illisible est **sauté** plutôt que fatal : l'inventaire
/// reste consultable quand une base est verrouillée par une autre instance.
#[must_use]
pub fn lister(paths: &Paths) -> Vec<Carte> {
    let mut cartes = Vec::new();
    for classeur in paths.classeurs_existants() {
        if let Ok(mut lot) = lire_classeur(paths, &classeur) {
            cartes.append(&mut lot);
        }
    }
    numeroter(&mut cartes);
    cartes
}

/// Lit les cartes possédées d'un classeur.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn lire_classeur(paths: &Paths, classeur: &str) -> Result<Vec<Carte>> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(classeur))
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let mut requete = conn
        .prepare(
            "SELECT rowid, name, name_fr, set_name, set_code, rarity,
                    quantite, qualite, extended_art, card_image_id
               FROM cards
              WHERE possessed = 1",
        )
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    let cartes = requete
        .query_map([], |l| {
            let quantite: Option<i64> = l.get(6)?;
            Ok(Carte {
                classeur: classeur.to_owned(),
                rowid: l.get(0)?,
                nom: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                nom_fr: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                set_nom: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
                set_code: l.get::<_, Option<String>>(4)?.unwrap_or_default(),
                rarete: l.get::<_, Option<String>>(5)?.unwrap_or_default(),
                // Une carte possédée compte pour au moins un exemplaire :
                // c'est ce que « possédée » veut dire.
                quantite: quantite.filter(|q| *q > 0).unwrap_or(1),
                qualite: l.get::<_, Option<String>>(7)?.unwrap_or_default(),
                overframe: l.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
                card_image_id: l.get(9)?,
                variantes: 1,
                variante: 0,
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| AppError::Creation(format!("{classeur} : {e}")))?;
    Ok(cartes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn carte(classeur: &str, rowid: i64, code: &str, rarete: &str, quantite: i64) -> Carte {
        Carte {
            classeur: classeur.to_owned(),
            rowid,
            nom: format!("Card {rowid}"),
            nom_fr: format!("Carte {rowid}"),
            set_nom: format!("Set {classeur}"),
            set_code: code.to_owned(),
            rarete: rarete.to_owned(),
            quantite,
            qualite: "NM".into(),
            overframe: false,
            card_image_id: Some(rowid),
            variantes: 1,
            variante: 0,
        }
    }

    /// Le playset et son surplus.
    #[test]
    fn le_playset_est_atteint_a_trois_et_le_surplus_commence_apres() {
        let attendu = [(0, false, 0), (2, false, 0), (3, true, 0), (5, true, 2)];
        for (quantite, playset, surplus) in attendu {
            let c = carte("RA02", 1, "RA02-EN001", "Common", quantite);
            assert_eq!(c.playset(), playset, "quantité {quantite}");
            assert_eq!(c.surplus(), surplus, "quantité {quantite}");
        }
    }

    /// La rareté fait partie du groupe de variantes.
    ///
    /// Sans elle, les sept tirages d'une carte de `RA02` — sept raretés,
    /// sept images — passeraient pour sept illustrations de la même carte,
    /// et l'inventaire afficherait « Art 1/7 » sur des lignes qui ne
    /// partagent rien.
    #[test]
    fn la_rarete_separe_les_groupes_de_variantes() {
        let mut cartes = vec![
            carte("RA02", 1, "RA02-EN001", "Collector's Rare", 1),
            carte("RA02", 2, "RA02-EN001", "Super Rare", 1),
            carte("RA02", 3, "RA02-EN001", "Ultra Rare", 1),
        ];
        numeroter(&mut cartes);
        for c in &cartes {
            assert_eq!(c.variantes, 1, "{} est seule de sa rareté", c.rarete);
            assert_eq!(c.variante, 0, "une seule variante ne se numérote pas");
        }
    }

    /// Deux illustrations d'un même tirage et d'une même rareté, elles,
    /// se numérotent — dans un ordre déterministe.
    #[test]
    fn deux_illustrations_d_une_meme_rarete_se_numerotent() {
        let mut cartes = vec![
            {
                let mut c = carte("RA02", 1, "RA02-EN001", "Secret Rare", 1);
                c.card_image_id = Some(14_878_871);
                c
            },
            {
                let mut c = carte("RA02", 2, "RA02-EN001", "Secret Rare", 1);
                c.card_image_id = Some(-1_276_398_914);
                c
            },
        ];
        numeroter(&mut cartes);
        assert_eq!(cartes[0].variantes, 2);
        assert_eq!(cartes[1].variantes, 2);
        // L'identifiant le plus petit vient en premier.
        assert_eq!(cartes[1].variante, 1, "l'identifiant négatif est le rang 1");
        assert_eq!(cartes[0].variante, 2);

        // L'Overframe passe après le cadre normal, à identifiant égal.
        let mut avec_overframe = vec![
            {
                let mut c = carte("RA05", 1, "RA05-EN001", "Secret Rare", 1);
                c.overframe = true;
                c.card_image_id = Some(7);
                c
            },
            {
                let mut c = carte("RA05", 2, "RA05-EN001", "Secret Rare", 1);
                c.card_image_id = Some(9);
                c
            },
        ];
        numeroter(&mut avec_overframe);
        assert_eq!(
            avec_overframe[1].variante, 1,
            "le cadre normal passe devant, malgré un identifiant plus grand"
        );
        assert_eq!(avec_overframe[0].variante, 2);
    }

    /// Chaque critère de filtre retient bien ce qu'il annonce, et rien
    /// d'autre.
    #[test]
    fn chaque_critere_filtre_ce_qu_il_annonce() {
        let mut cartes = vec![
            carte("RA02", 1, "RA02-EN001", "Common", 1),
            carte("RA02", 2, "RA02-EN002", "Secret Rare", 3),
            carte("SDLI", 3, "SDLI-EN001", "Common", 5),
        ];
        cartes[2].qualite = String::new();
        cartes[1].nom_fr = "Dragon Blanc".into();

        let cas: [(Filtre, Vec<i64>); 7] = [
            (Filtre::default(), vec![1, 2, 3]),
            (
                Filtre {
                    classeur: "SDLI".into(),
                    ..Filtre::default()
                },
                vec![3],
            ),
            (
                Filtre {
                    rarete: "Common".into(),
                    ..Filtre::default()
                },
                vec![1, 3],
            ),
            (
                Filtre {
                    code: "en00".into(),
                    ..Filtre::default()
                },
                vec![1, 2, 3],
            ),
            (
                Filtre {
                    nom: "dragon".into(),
                    ..Filtre::default()
                },
                vec![2],
            ),
            (
                Filtre {
                    qualite: Filtre::SANS_QUALITE.into(),
                    ..Filtre::default()
                },
                vec![3],
            ),
            (
                Filtre {
                    sous_playset: true,
                    ..Filtre::default()
                },
                vec![1],
            ),
        ];
        for (filtre, attendu) in cas {
            let vus: Vec<i64> = filtrer(&cartes, &filtre, true)
                .iter()
                .map(|c| c.rowid)
                .collect();
            assert_eq!(vus, attendu, "{filtre:?}");
        }
    }

    /// Un état vide se demande explicitement — la chaîne vide veut dire
    /// « ne pas filtrer », et les deux ne peuvent pas être le même mot.
    #[test]
    fn l_etat_vide_se_demande_par_un_mot_a_lui() {
        let mut cartes = vec![carte("RA02", 1, "RA02-EN001", "Common", 1)];
        cartes[0].qualite = String::new();
        let sans_filtre = Filtre::default();
        assert!(!sans_filtre.actif());
        assert_eq!(filtrer(&cartes, &sans_filtre, true).len(), 1);

        let vide = Filtre {
            qualite: Filtre::SANS_QUALITE.into(),
            ..Filtre::default()
        };
        assert!(vide.actif());
        assert_eq!(filtrer(&cartes, &vide, true).len(), 1);

        cartes[0].qualite = "NM".into();
        assert_eq!(
            filtrer(&cartes, &vide, true).len(),
            0,
            "une carte en état renseigné n'est pas « sans état »"
        );
    }

    /// Le filtre par nom suit la langue affichée.
    #[test]
    fn le_filtre_par_nom_suit_la_langue() {
        let mut c = carte("RA02", 1, "RA02-EN001", "Common", 1);
        c.nom = "Rescue Cat".into();
        c.nom_fr = "Chat Sauveteur".into();
        let cartes = vec![c];
        let chat = Filtre {
            nom: "chat".into(),
            ..Filtre::default()
        };
        assert_eq!(filtrer(&cartes, &chat, true).len(), 1);
        assert_eq!(filtrer(&cartes, &chat, false).len(), 0, "en anglais, non");
        let rescue = Filtre {
            nom: "rescue".into(),
            ..Filtre::default()
        };
        assert_eq!(filtrer(&cartes, &rescue, false).len(), 1);
    }

    /// Le tri est total : à valeur égale, l'ordre reste le même d'un appel
    /// à l'autre.
    #[test]
    fn le_tri_departage_les_ex_aequo() {
        let mut cartes = vec![
            carte("SDLI", 9, "SDLI-EN009", "Common", 1),
            carte("RA02", 2, "RA02-EN002", "Common", 1),
            carte("RA02", 1, "RA02-EN001", "Common", 1),
        ];
        // Toutes les quantités sont égales : seul le départage décide.
        trier(&mut cartes, Colonne::Quantite, false, true);
        let ordre: Vec<(String, i64)> = cartes
            .iter()
            .map(|c| (c.classeur.clone(), c.rowid))
            .collect();
        assert_eq!(
            ordre,
            vec![
                ("RA02".to_owned(), 1),
                ("RA02".to_owned(), 2),
                ("SDLI".to_owned(), 9)
            ]
        );
        // Et le sens descendant renverse la colonne sans casser le
        // départage.
        let mut autre = cartes.clone();
        trier(&mut autre, Colonne::Quantite, true, true);
        assert_eq!(autre, cartes, "à quantité égale, l'ordre ne bouge pas");
    }

    /// Le tri descendant inverse bien la colonne demandée.
    #[test]
    fn le_tri_descendant_inverse_la_colonne() {
        let mut cartes = vec![
            carte("RA02", 1, "RA02-EN001", "Common", 1),
            carte("RA02", 2, "RA02-EN002", "Common", 5),
            carte("RA02", 3, "RA02-EN003", "Common", 3),
        ];
        trier(&mut cartes, Colonne::Quantite, false, true);
        assert_eq!(
            cartes.iter().map(|c| c.quantite).collect::<Vec<_>>(),
            vec![1, 3, 5]
        );
        trier(&mut cartes, Colonne::Quantite, true, true);
        assert_eq!(
            cartes.iter().map(|c| c.quantite).collect::<Vec<_>>(),
            vec![5, 3, 1]
        );
        // Le surplus n'est pas la quantité : 1 et 3 ont le même surplus.
        trier(&mut cartes, Colonne::Surplus, true, true);
        assert_eq!(
            cartes.iter().map(|c| c.surplus()).collect::<Vec<_>>(),
            vec![2, 0, 0]
        );
    }

    /// Les totaux comptent les lignes, les exemplaires et les playsets —
    /// trois chiffres différents.
    #[test]
    fn les_totaux_distinguent_lignes_exemplaires_et_playsets() {
        let cartes = [
            carte("RA02", 1, "RA02-EN001", "Common", 1),
            carte("RA02", 2, "RA02-EN002", "Common", 3),
            carte("RA02", 3, "RA02-EN003", "Common", 5),
        ];
        let vus: Vec<&Carte> = cartes.iter().collect();
        let t = totaux(&vus);
        assert_eq!(t.lignes, 3);
        assert_eq!(t.exemplaires, 9, "1 + 3 + 5");
        assert_eq!(t.playsets, 2, "seules deux atteignent trois exemplaires");
        assert_eq!(t.surplus, 2, "seule la dernière dépasse, de deux");
    }

    /// Les valeurs proposées au filtre sont exactement celles qui existent.
    #[test]
    fn les_valeurs_proposees_sont_celles_qui_existent() {
        let mut cartes = vec![
            carte("RA02", 1, "RA02-EN001", "Common", 1),
            carte("RA02", 2, "RA02-EN002", "Secret Rare", 1),
            carte("SDLI", 3, "SDLI-EN001", "Common", 1),
        ];
        cartes[2].rarete = String::new();
        let raretes = valeurs(&cartes, |c| &c.rarete);
        assert_eq!(
            raretes,
            vec!["Common".to_owned(), "Secret Rare".to_owned()],
            "triées, dédoublonnées, et sans la valeur vide"
        );
        assert_eq!(
            valeurs(&cartes, |c| &c.classeur),
            vec!["RA02".to_owned(), "SDLI".to_owned()]
        );
    }

    /// Une installation jetable avec un classeur au vrai schéma.
    fn installation(lignes: &[(i64, &str, i64, i64)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (rowid, set_code, possedee, quantite) in lignes {
            conn.execute(
                "INSERT INTO cards
                    (rowid, name, name_fr, set_name, set_code, rarity,
                     possessed, quantite, card_image_id)
                 VALUES (?1, 'Rescue Cat', 'Chat Sauveteur', 'Rarity 02', ?2,
                         'Secret Rare', ?3, ?4, ?1)",
                rusqlite::params![rowid, set_code, possedee, quantite],
            )
            .unwrap();
        }
        (tmp, paths)
    }

    /// L'inventaire ne montre que ce qui est possédé.
    #[test]
    fn seules_les_cartes_possedees_entrent_dans_l_inventaire() {
        let (_tmp, paths) = installation(&[
            (1, "RA02-EN001", 1, 2),
            (2, "RA02-EN002", 0, 0),
            (3, "RA02-EN003", 1, 4),
        ]);
        let cartes = lister(&paths);
        let vus: Vec<i64> = cartes.iter().map(|c| c.rowid).collect();
        assert_eq!(vus, vec![1, 3]);
        assert_eq!(cartes[0].nom, "Rescue Cat");
        assert_eq!(cartes[0].nom_fr, "Chat Sauveteur");
        assert_eq!(cartes[0].classeur, "RA02");
    }

    /// Une carte possédée dont la quantité est nulle ou absente compte
    /// pour **un** exemplaire.
    ///
    /// # Pourquoi cette garde existe
    ///
    /// Les classeurs d'avant l'import CSV portent `possessed = 1` sans
    /// quantité renseignée — c'est ce que fait la saisie au clic dans le
    /// classeur. Sans le repli, ces cartes apparaîtraient à zéro
    /// exemplaire : l'inventaire annoncerait « 0 exemplaire » sur une
    /// carte qu'on a en main, et les totaux compteraient faux.
    #[test]
    fn une_carte_possedee_sans_quantite_compte_pour_un() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 1, 0)]);
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        conn.execute("UPDATE cards SET quantite = NULL WHERE rowid = 1", ())
            .unwrap();
        drop(conn);

        let cartes = lister(&paths);
        assert_eq!(cartes.len(), 1);
        assert_eq!(
            cartes[0].quantite, 1,
            "possédée sans quantité, donc un exemplaire"
        );
        let vus: Vec<&Carte> = cartes.iter().collect();
        assert_eq!(totaux(&vus).exemplaires, 1);
    }

    /// Un classeur illisible est sauté, les autres restent consultables.
    #[test]
    fn un_classeur_illisible_ne_vide_pas_l_inventaire() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 1, 1)]);
        // Un dossier de classeur sans base : `classeurs_existants` le voit,
        // la lecture échoue, l'inventaire continue.
        std::fs::create_dir_all(paths.dossier_classeur("VIDE")).unwrap();
        std::fs::write(paths.classeur_db("VIDE"), b"ceci n'est pas une base").unwrap();
        let cartes = lister(&paths);
        assert_eq!(cartes.len(), 1, "RA02 reste lisible");
    }

    /// L'écriture en masse touche exactement les lignes désignées.
    #[test]
    fn l_ecriture_en_masse_touche_les_lignes_designees_et_pas_les_autres() {
        let (_tmp, paths) = installation(&[
            (1, "RA02-EN001", 1, 1),
            (2, "RA02-EN002", 1, 1),
            (3, "RA02-EN003", 1, 1),
        ]);
        let cibles = vec![("RA02".to_owned(), 1), ("RA02".to_owned(), 3)];
        let touchees = definir_qualite(&paths, &cibles, "NM").unwrap();
        assert_eq!(touchees.lignes, 2);
        assert_eq!(touchees.classeurs, vec!["RA02".to_owned()]);

        let etats: Vec<(i64, String)> = lister(&paths)
            .iter()
            .map(|c| (c.rowid, c.qualite.clone()))
            .collect();
        assert_eq!(
            etats,
            vec![
                (1, "NM".to_owned()),
                (2, String::new()),
                (3, "NM".to_owned())
            ],
            "la ligne 2 n'était pas visée"
        );
    }

    /// Un état vide **efface** — c'est le retour en arrière après un lot
    /// mal choisi.
    #[test]
    fn un_etat_vide_efface_au_lieu_de_ne_rien_faire() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 1, 1)]);
        let cibles = vec![("RA02".to_owned(), 1)];
        definir_qualite(&paths, &cibles, "NM").unwrap();
        assert_eq!(lister(&paths)[0].qualite, "NM");
        definir_qualite(&paths, &cibles, "   ").unwrap();
        assert_eq!(lister(&paths)[0].qualite, "", "les espaces sont rognés");
    }

    /// L'édition s'écrit aussi — c'est elle qui décide de la colonne à
    /// l'export.
    #[test]
    fn l_edition_s_ecrit_en_masse() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 1, 1)]);
        let cibles = vec![("RA02".to_owned(), 1)];
        definir_edition(&paths, &cibles, "unlimited").unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let lu: String = conn
            .query_row("SELECT edition FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(lu, "unlimited");
    }

    /// Une quantité nulle retire la carte de l'inventaire sans effacer sa
    /// ligne : le tirage existe toujours, il n'est simplement plus possédé.
    #[test]
    fn une_quantite_nulle_retire_sans_effacer_la_ligne() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 1, 3), (2, "RA02-EN002", 1, 1)]);
        let cibles = vec![("RA02".to_owned(), 1)];
        retirer(&paths, &cibles).unwrap();

        let restants = lister(&paths);
        assert_eq!(restants.len(), 1, "elle a quitté l'inventaire");
        assert_eq!(restants[0].rowid, 2);

        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let total: i64 = conn
            .query_row("SELECT count(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(total, 2, "mais sa ligne est toujours là");
    }

    /// Fixer une quantité rend la carte possédée, et le fait dans les deux
    /// sens.
    #[test]
    fn la_quantite_et_la_possession_vont_ensemble() {
        let (_tmp, paths) = installation(&[(1, "RA02-EN001", 0, 0)]);
        assert!(lister(&paths).is_empty(), "rien de possédé au départ");
        let cibles = vec![("RA02".to_owned(), 1)];
        definir_quantite(&paths, &cibles, 4).unwrap();
        let cartes = lister(&paths);
        assert_eq!(cartes.len(), 1);
        assert_eq!(cartes[0].quantite, 4);
        assert!(cartes[0].playset());

        // Une quantité négative ne descend pas sous zéro.
        //
        // # Ce test n'a d'abord rien vérifié
        //
        // Il se contentait de constater que la carte quittait l'inventaire.
        // Or elle le quitte de toute façon : `possessed` suit `quantite > 0`,
        // vrai pour −5 comme pour 0. Une mutation retirant le plancher
        // survivait donc. Ce qui compte est la **valeur stockée** : un −5 en
        // base est un mensonge que tout autre lecteur — l'écran classeur,
        // les statistiques, une requête SQL à la main — prendrait au mot.
        definir_quantite(&paths, &cibles, -5).unwrap();
        assert!(lister(&paths).is_empty());
        let conn = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let stockee: i64 = conn
            .query_row("SELECT quantite FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(stockee, 0, "et non −5");
    }

    /// Un lot réparti sur plusieurs classeurs les touche tous, et les
    /// nomme.
    #[test]
    fn un_lot_sur_plusieurs_classeurs_les_nomme_tous() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        for code in ["RA02", "SDLI"] {
            std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
            let conn = rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
            conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite)
                 VALUES (1, 'Carte', ?1, 'Common', 1, 1)",
                [format!("{code}-EN001")],
            )
            .unwrap();
        }
        let cibles = vec![("SDLI".to_owned(), 1), ("RA02".to_owned(), 1)];
        let touchees = definir_qualite(&paths, &cibles, "EX").unwrap();
        assert_eq!(touchees.lignes, 2);
        assert_eq!(
            touchees.classeurs,
            vec!["RA02".to_owned(), "SDLI".to_owned()],
            "nommés dans l'ordre, quel que soit l'ordre des cibles"
        );
        assert!(lister(&paths).iter().all(|c| c.qualite == "EX"));
    }

    /// Les colonnes ont toutes un en-tête, et tous distincts.
    #[test]
    fn chaque_colonne_a_son_propre_en_tete() {
        let libelles: std::collections::HashSet<&str> =
            Colonne::toutes().iter().map(|c| c.libelle()).collect();
        assert_eq!(libelles.len(), Colonne::toutes().len());
    }
}
