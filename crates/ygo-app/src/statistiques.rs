// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les statistiques de la collection : où en est chaque classeur, et sur
//! quelles raretés le retard se concentre.
//!
//! Portage de `module/statistique/statistique_collection_service.py` pour
//! les chiffres, et de `panneau_rarete_classeur.agreger_par_rarete` pour
//! leur ordre.
//!
//! # Deux définitions de « possédée » cohabitaient
//!
//! Le service compte `possessed = 1`. Le panneau par rareté, lui, compte
//! `quantite > 0`. Les deux vivent dans la même fonctionnalité et rien ne
//! garantit qu'elles s'accordent — une ligne marquée possédée à quantité
//! nulle serait comptée par l'un et pas par l'autre.
//!
//! Vérification faite sur les huit classeurs réels : **zéro ligne**
//! diverge, dans un sens comme dans l'autre. C'est `possessed = 1` qui est
//! retenu ici, parce que c'est la définition du service, celle dont les
//! chiffres sont figés dans l'oracle. La garde de [`crate::import`] et
//! celle de [`crate::inventaire`] maintiennent l'accord à l'écriture.
//!
//! # Une rareté vide compte dans le total mais pas dans le détail
//!
//! Le SQL du Python écarte `rarity IS NULL`, puis le code écarte encore la
//! chaîne vide. Une ligne sans rareté est donc comptée dans le total du
//! classeur et absente de sa décomposition — les deux ne s'additionnent
//! pas forcément. C'est reproduit tel quel (règle R9) : aucun classeur réel
//! n'est dans ce cas aujourd'hui, mais un classeur bricolé le serait, et
//! deux totaux qui ne se recoupent pas valent mieux qu'un chiffre inventé.

use ygo_core::paths::Paths;
use ygo_core::rarity::reference;

use crate::error::{AppError, Result};

/// Ce qu'une rareté représente dans un classeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParRarete {
    /// Le libellé, tel qu'il est en base.
    pub rarete: String,
    /// Nombre de lignes de cette rareté.
    pub total: usize,
    /// Combien sont possédées.
    pub possedees: usize,
}

impl ParRarete {
    /// La part possédée, de 0 à 100.
    #[must_use]
    pub fn pourcentage(&self) -> f64 {
        part(self.possedees, self.total)
    }

    /// Combien il en manque.
    #[must_use]
    pub fn manquantes(&self) -> usize {
        self.total.saturating_sub(self.possedees)
    }
}

/// L'état d'un classeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classeur {
    /// Le code du classeur.
    pub nom: String,
    /// Nombre de lignes.
    pub total: usize,
    /// Combien sont possédées.
    pub possedees: usize,
    /// La décomposition par rareté, **dans l'ordre du référentiel**.
    pub raretes: Vec<ParRarete>,
}

impl Classeur {
    /// La part possédée, de 0 à 100.
    #[must_use]
    pub fn pourcentage(&self) -> f64 {
        part(self.possedees, self.total)
    }

    /// Combien il manque de cartes.
    #[must_use]
    pub fn manquantes(&self) -> usize {
        self.total.saturating_sub(self.possedees)
    }

    /// Le classeur est-il complet ?
    #[must_use]
    pub fn complet(&self) -> bool {
        self.total > 0 && self.possedees >= self.total
    }
}

/// Les chiffres de toute la collection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totaux {
    /// Nombre de classeurs.
    pub classeurs: usize,
    /// Nombre de lignes, tous classeurs confondus.
    pub total: usize,
    /// Combien sont possédées.
    pub possedees: usize,
    /// Combien de classeurs sont complets.
    pub complets: usize,
}

impl Totaux {
    /// La part possédée, de 0 à 100.
    #[must_use]
    pub fn pourcentage(&self) -> f64 {
        part(self.possedees, self.total)
    }
}

/// Une part en pourcentage, sans division par zéro.
fn part(numerateur: usize, denominateur: usize) -> f64 {
    if denominateur == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        numerateur as f64 / denominateur as f64 * 100.0
    }
}

/// Le rang d'une rareté dans l'ordre du référentiel.
///
/// L'ordre est celui d'insertion de la table du Python — Commune en tête,
/// Ghost Rare en queue — et non l'ordre alphabétique. Une rareté que le
/// référentiel ne connaît pas passe **en dernier** plutôt qu'au hasard.
///
/// ```
/// use ygo_app::statistiques::rang_rarete;
/// assert!(rang_rarete("Common") < rang_rarete("Secret Rare"));
/// assert!(rang_rarete("Secret Rare") < rang_rarete("Ghost Rare"));
/// assert!(rang_rarete("Rareté Martienne") > rang_rarete("Ghost Rare"));
/// ```
#[must_use]
pub fn rang_rarete(nom: &str) -> usize {
    let inconnu = reference::table().len() + 1;
    let Some(code) = reference::nom_vers_code(nom) else {
        return inconnu;
    };
    reference::table()
        .iter()
        .position(|r| r.code == code)
        .unwrap_or(inconnu)
}

/// Range les raretés : par ordre du référentiel, puis par libellé.
///
/// Le second critère n'est pas décoratif : quatorze clés du référentiel
/// sont en collision, et plusieurs libellés peuvent donc partager un rang.
/// Sans départage, leur ordre dépendrait de celui du parcours de la base.
pub fn ordonner(raretes: &mut [ParRarete]) {
    raretes.sort_by(|a, b| {
        rang_rarete(&a.rarete)
            .cmp(&rang_rarete(&b.rarete))
            .then_with(|| a.rarete.to_lowercase().cmp(&b.rarete.to_lowercase()))
    });
}

/// Additionne les classeurs.
#[must_use]
pub fn totaux(classeurs: &[Classeur]) -> Totaux {
    Totaux {
        classeurs: classeurs.len(),
        total: classeurs.iter().map(|c| c.total).sum(),
        possedees: classeurs.iter().map(|c| c.possedees).sum(),
        complets: classeurs.iter().filter(|c| c.complet()).count(),
    }
}

/// Ne garde que les classeurs dont le nom contient `terme`.
///
/// Sans tenir compte de la casse ; un terme vide ne filtre pas.
#[must_use]
pub fn filtrer<'a>(classeurs: &'a [Classeur], terme: &str) -> Vec<&'a Classeur> {
    let q = terme.trim().to_lowercase();
    classeurs
        .iter()
        .filter(|c| q.is_empty() || c.nom.to_lowercase().contains(&q))
        .collect()
}

/// Les raretés de toute la collection, additionnées entre classeurs.
///
/// Ce que le Python ne calculait pas : il montrait la décomposition
/// classeur par classeur, jamais l'ensemble. C'est pourtant la vue qui dit
/// où le retard se concentre — cinq Secret Rare manquantes réparties sur
/// cinq classeurs ne se voient nulle part autrement.
#[must_use]
pub fn raretes_cumulees(classeurs: &[Classeur]) -> Vec<ParRarete> {
    let mut cumul: std::collections::BTreeMap<&str, (usize, usize)> =
        std::collections::BTreeMap::new();
    for c in classeurs {
        for r in &c.raretes {
            let e = cumul.entry(r.rarete.as_str()).or_default();
            e.0 += r.total;
            e.1 += r.possedees;
        }
    }
    let mut v: Vec<ParRarete> = cumul
        .into_iter()
        .map(|(rarete, (total, possedees))| ParRarete {
            rarete: rarete.to_owned(),
            total,
            possedees,
        })
        .collect();
    ordonner(&mut v);
    v
}

/// Lit les statistiques d'un classeur.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn lire_classeur(paths: &Paths, code: &str) -> Result<Classeur> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(code))
        .map_err(|e| AppError::Creation(format!("{code} : {e}")))?;
    let (total, possedees) = conn
        .query_row(
            "SELECT COUNT(*), SUM(CASE WHEN possessed = 1 THEN 1 ELSE 0 END) FROM cards",
            [],
            |l| {
                Ok((
                    l.get::<_, i64>(0)?,
                    l.get::<_, Option<i64>>(1)?.unwrap_or(0),
                ))
            },
        )
        .map_err(|e| AppError::Creation(format!("{code} : {e}")))?;

    let mut requete = conn
        .prepare(
            "SELECT rarity, COUNT(*),
                    SUM(CASE WHEN possessed = 1 THEN 1 ELSE 0 END)
               FROM cards
              WHERE rarity IS NOT NULL
              GROUP BY rarity",
        )
        .map_err(|e| AppError::Creation(format!("{code} : {e}")))?;
    let mut raretes: Vec<ParRarete> = requete
        .query_map([], |l| {
            Ok(ParRarete {
                rarete: l.get::<_, String>(0)?,
                total: usize::try_from(l.get::<_, i64>(1)?).unwrap_or(0),
                possedees: usize::try_from(l.get::<_, Option<i64>>(2)?.unwrap_or(0)).unwrap_or(0),
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|e| AppError::Creation(format!("{code} : {e}")))?
        // Le Python écarte encore la chaîne vide après le SQL : une rareté
        // vide compte dans le total, pas dans le détail.
        .into_iter()
        .filter(|r| !r.rarete.is_empty())
        .collect();
    ordonner(&mut raretes);

    Ok(Classeur {
        nom: code.to_owned(),
        total: usize::try_from(total).unwrap_or(0),
        possedees: usize::try_from(possedees).unwrap_or(0),
        raretes,
    })
}

/// Lit les statistiques de toute l'installation, triées par nom.
///
/// Le tri vient de [`Paths::classeurs_existants`], qui rend déjà ses codes
/// ordonnés. Re-trier ici serait un doublon : une mutation retirant ce
/// second tri survivrait à tous les tests, ce qui est la définition d'un
/// code mort. La garantie tient donc à un seul endroit, et
/// `les_classeurs_sortent_tries` la surveille — si `classeurs_existants`
/// cessait de trier, ce test le dirait.
///
/// Un classeur illisible est **sauté** plutôt que fatal, comme dans le
/// Python : les statistiques restent consultables quand une base est
/// verrouillée par une autre instance.
#[must_use]
pub fn lister(paths: &Paths) -> Vec<Classeur> {
    paths
        .classeurs_existants()
        .iter()
        .filter_map(|c| lire_classeur(paths, c).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use std::collections::HashMap;

    fn rarete(nom: &str, total: usize, possedees: usize) -> ParRarete {
        ParRarete {
            rarete: nom.to_owned(),
            total,
            possedees,
        }
    }

    fn classeur(nom: &str, total: usize, possedees: usize) -> Classeur {
        Classeur {
            nom: nom.to_owned(),
            total,
            possedees,
            raretes: Vec::new(),
        }
    }

    /// Les pourcentages, y compris le cas du classeur vide.
    #[test]
    fn un_classeur_vide_ne_divise_pas_par_zero() {
        assert!((classeur("A", 0, 0).pourcentage() - 0.0).abs() < 1e-9);
        assert!((classeur("A", 4, 1).pourcentage() - 25.0).abs() < 1e-9);
        assert!((classeur("A", 3, 3).pourcentage() - 100.0).abs() < 1e-9);
        assert!(!classeur("A", 0, 0).complet(), "vide n'est pas complet");
        assert!(classeur("A", 3, 3).complet());
        assert!(!classeur("A", 3, 2).complet());
        assert_eq!(classeur("A", 10, 3).manquantes(), 7);
    }

    /// L'ordre des raretés est celui du référentiel, pas l'alphabet.
    #[test]
    fn les_raretes_suivent_l_ordre_du_referentiel() {
        let mut v = vec![
            rarete("Ghost Rare", 1, 0),
            rarete("Common", 10, 5),
            rarete("Secret Rare", 3, 1),
            rarete("Super Rare", 4, 2),
        ];
        ordonner(&mut v);
        let noms: Vec<&str> = v.iter().map(|r| r.rarete.as_str()).collect();
        assert_eq!(
            noms,
            vec!["Common", "Super Rare", "Secret Rare", "Ghost Rare"],
            "l'ordre alphabétique aurait donné Common, Ghost, Secret, Super"
        );
    }

    /// Une rareté inconnue du référentiel passe en dernier, et les
    /// inconnues se départagent entre elles par leur libellé.
    #[test]
    fn les_raretes_inconnues_ferment_la_marche() {
        let mut v = vec![
            rarete("Zèbre Rare", 1, 0),
            rarete("Common", 1, 0),
            rarete("Abricot Rare", 1, 0),
        ];
        ordonner(&mut v);
        let noms: Vec<&str> = v.iter().map(|r| r.rarete.as_str()).collect();
        assert_eq!(noms, vec!["Common", "Abricot Rare", "Zèbre Rare"]);
    }

    /// Les totaux comptent quatre choses différentes.
    #[test]
    fn les_totaux_distinguent_ce_qu_il_faut() {
        let cs = vec![
            classeur("EGO1", 35, 35),
            classeur("RA02", 567, 21),
            classeur("VIDE", 0, 0),
        ];
        let t = totaux(&cs);
        assert_eq!(t.classeurs, 3);
        assert_eq!(t.total, 602);
        assert_eq!(t.possedees, 56);
        assert_eq!(t.complets, 1, "seul EGO1 ; le vide ne compte pas");
        assert!((t.pourcentage() - 9.302_325_581_395_35).abs() < 1e-9);
    }

    /// Le filtre porte sur le nom, sans tenir compte de la casse.
    #[test]
    fn le_filtre_cherche_dans_le_nom() {
        let cs = vec![
            classeur("EGO1", 1, 0),
            classeur("EGS1", 1, 0),
            classeur("RA02", 1, 0),
        ];
        assert_eq!(filtrer(&cs, "").len(), 3, "vide ne filtre pas");
        assert_eq!(filtrer(&cs, "  ").len(), 3);
        assert_eq!(filtrer(&cs, "eg").len(), 2);
        assert_eq!(filtrer(&cs, "RA").len(), 1);
        assert_eq!(filtrer(&cs, "ra02").len(), 1, "la casse est indifférente");
        assert!(filtrer(&cs, "zzz").is_empty());
    }

    /// Le cumul par rareté additionne entre classeurs — c'est la vue que
    /// le Python n'avait pas.
    #[test]
    fn le_cumul_additionne_les_classeurs() {
        let cs = vec![
            Classeur {
                nom: "A".into(),
                total: 3,
                possedees: 2,
                raretes: vec![rarete("Common", 2, 2), rarete("Secret Rare", 1, 0)],
            },
            Classeur {
                nom: "B".into(),
                total: 5,
                possedees: 1,
                raretes: vec![rarete("Common", 4, 1), rarete("Ghost Rare", 1, 0)],
            },
        ];
        let cumul = raretes_cumulees(&cs);
        let noms: Vec<&str> = cumul.iter().map(|r| r.rarete.as_str()).collect();
        assert_eq!(
            noms,
            vec!["Common", "Secret Rare", "Ghost Rare"],
            "et dans l'ordre du référentiel"
        );
        assert_eq!(cumul[0].total, 6, "2 + 4");
        assert_eq!(cumul[0].possedees, 3, "2 + 1");
        assert_eq!(cumul[1].manquantes(), 1);
        assert!(raretes_cumulees(&[]).is_empty());
    }

    /// Une installation jetable, au vrai schéma.
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
            let conn = rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
            conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            for (i, (rarete, possedee)) in cartes.iter().enumerate() {
                conn.execute(
                    "INSERT INTO cards (name, set_code, rarity, possessed, quantite)
                     VALUES ('Carte', ?1, ?2, ?3, ?3)",
                    rusqlite::params![format!("{code}-EN{i:03}"), rarete, possedee],
                )
                .unwrap();
            }
        }
        (tmp, paths)
    }

    /// La lecture d'un classeur : totaux et décomposition.
    #[test]
    fn un_classeur_se_lit_avec_sa_decomposition() {
        let (_tmp, paths) = installation(&[
            ("RA02", "Common", 1),
            ("RA02", "Common", 0),
            ("RA02", "Secret Rare", 1),
        ]);
        let c = lire_classeur(&paths, "RA02").unwrap();
        assert_eq!(c.nom, "RA02");
        assert_eq!(c.total, 3);
        assert_eq!(c.possedees, 2);
        assert_eq!(c.raretes.len(), 2);
        assert_eq!(c.raretes[0], rarete("Common", 2, 1));
        assert_eq!(c.raretes[1], rarete("Secret Rare", 1, 1));
    }

    /// Une rareté vide compte dans le total et non dans le détail — le
    /// comportement du Python, reproduit tel quel.
    #[test]
    fn une_rarete_vide_compte_dans_le_total_pas_dans_le_detail() {
        let (_tmp, paths) = installation(&[("RA02", "Common", 1), ("RA02", "", 1)]);
        let c = lire_classeur(&paths, "RA02").unwrap();
        assert_eq!(c.total, 2, "les deux lignes comptent");
        assert_eq!(c.possedees, 2);
        assert_eq!(c.raretes.len(), 1, "mais une seule rareté est détaillée");
        assert_eq!(
            c.raretes.iter().map(|r| r.total).sum::<usize>(),
            1,
            "le détail ne se recoupe donc pas avec le total"
        );
    }

    /// Un classeur illisible ne fait pas tomber les statistiques.
    #[test]
    fn un_classeur_illisible_est_saute() {
        let (_tmp, paths) = installation(&[("RA02", "Common", 1)]);
        std::fs::create_dir_all(paths.dossier_classeur("CASSE")).unwrap();
        std::fs::write(paths.classeur_db("CASSE"), b"pas une base").unwrap();
        let cs = lister(&paths);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].nom, "RA02");
    }

    /// Les classeurs sortent triés par nom.
    ///
    /// Le tri est celui de `classeurs_existants` : ce test est la garde qui
    /// s'assure qu'il tient toujours, puisque `lister` ne le refait pas.
    #[test]
    fn les_classeurs_sortent_tries() {
        let (_tmp, paths) = installation(&[
            ("VASM", "Common", 1),
            ("EGO1", "Common", 1),
            ("RA02", "Common", 1),
        ]);
        let noms: Vec<String> = lister(&paths).iter().map(|c| c.nom.clone()).collect();
        assert_eq!(noms, vec!["EGO1", "RA02", "VASM"]);
    }

    /// L'oracle : les chiffres du Python sur les huit classeurs réels.
    ///
    /// `bases.json` porte l'entrée — rareté, possession, quantité de chaque
    /// ligne — et `attendu.json` la sortie de
    /// `statistique_collection_service.get_stats_collection` **exécuté pour
    /// de bon** sur ces bases. Le Rust rejoue la même entrée et doit rendre
    /// les mêmes chiffres.
    #[test]
    fn les_chiffres_du_python_sont_reproduits() {
        #[derive(serde::Deserialize)]
        struct LigneBrute {
            rarity: Option<String>,
            possessed: Option<i64>,
        }
        #[derive(serde::Deserialize)]
        struct RareteAttendue {
            total: usize,
            possedees: usize,
        }
        #[derive(serde::Deserialize)]
        struct ClasseurAttendu {
            nom: String,
            total: usize,
            possedees: usize,
            pourcentage: f64,
            raretes: HashMap<String, RareteAttendue>,
        }

        let bases: HashMap<String, Vec<LigneBrute>> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/oracle/statistiques/bases.json"
        ))
        .unwrap();
        let attendu: Vec<ClasseurAttendu> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/oracle/statistiques/attendu.json"
        ))
        .unwrap();
        assert_eq!(attendu.len(), 8, "huit classeurs figés");

        // On reconstruit une installation à partir de l'entrée figée, puis
        // on la lit avec le code de production — pas avec un calcul écrit
        // pour le test.
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        for (code, lignes) in &bases {
            std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
            let conn = rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
            conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            let mut insert = conn
                .prepare("INSERT INTO cards (name, rarity, possessed) VALUES ('C', ?1, ?2)")
                .unwrap();
            for l in lignes {
                insert
                    .execute(rusqlite::params![l.rarity, l.possessed.unwrap_or(0)])
                    .unwrap();
            }
        }

        let obtenu = lister(&paths);
        assert_eq!(obtenu.len(), attendu.len());
        for (a, o) in attendu.iter().zip(&obtenu) {
            assert_eq!(o.nom, a.nom);
            assert_eq!(o.total, a.total, "{} total", a.nom);
            assert_eq!(o.possedees, a.possedees, "{} possédées", a.nom);
            assert!(
                (o.pourcentage() - a.pourcentage).abs() < 1e-9,
                "{} pourcentage : {} contre {}",
                a.nom,
                o.pourcentage(),
                a.pourcentage
            );
            assert_eq!(
                o.raretes.len(),
                a.raretes.len(),
                "{} : {} raretés contre {}",
                a.nom,
                o.raretes.len(),
                a.raretes.len()
            );
            for r in &o.raretes {
                let attendue = a
                    .raretes
                    .get(&r.rarete)
                    .unwrap_or_else(|| panic!("{} : rareté {} inattendue", a.nom, r.rarete));
                assert_eq!(r.total, attendue.total, "{} {}", a.nom, r.rarete);
                assert_eq!(r.possedees, attendue.possedees, "{} {}", a.nom, r.rarete);
            }
        }

        // Et les totaux de la collection entière, que le Python affichait
        // en récapitulatif.
        let t = totaux(&obtenu);
        assert_eq!(t.classeurs, 8);
        assert_eq!(t.total, attendu.iter().map(|a| a.total).sum::<usize>());
        assert_eq!(
            t.possedees,
            attendu.iter().map(|a| a.possedees).sum::<usize>()
        );
    }
}
