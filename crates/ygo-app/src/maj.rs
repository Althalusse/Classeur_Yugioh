// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La mise à jour de la base de référence — « MAJ BDD ».
//!
//! Portage de `check_for_updates()` et de son enchaînement sur
//! `BDD_creation.run_init`.
//!
//! # Pourquoi ce module existe alors que [`crate::init`] existe déjà
//!
//! [`init::initialiser`] construit `bdd/cardinfo.db` — et rien d'autre. Il ne
//! touche pas à `bdd/last_update.txt`, qui est le seul endroit où
//! l'application retient **quelle** version elle possède. Tant que ce fichier
//! n'est pas écrit, [`ygo_core::version::mise_a_jour_necessaire`] répond
//! toujours « oui » : une base fraîchement construite se déclare périmée à la
//! seconde qui suit.
//!
//! Le Python ne séparait pas les deux : `check_for_updates()` interrogeait
//! l'API, lançait l'initialisation, puis écrivait le fichier de version dans
//! la foulée. Ce module rétablit cet enchaînement — et **seulement** lui : le
//! parsing, l'écriture en base et la progression restent dans [`crate::init`].
//!
//! # L'ordre compte
//!
//! La version n'est écrite qu'**après** que la base est construite et
//! vérifiée. Une écriture anticipée ferait mentir le fichier si
//! l'initialisation échouait à mi-chemin : l'application se croirait à jour
//! sur une base absente, et ne proposerait plus jamais de la refaire.
//!
//! # Ce qui n'est pas une erreur
//!
//! L'appel à `checkDBVer.php` peut échouer alors que l'archive YGOJSON, elle,
//! est arrivée. La base est alors bonne ; seule la version reste inconnue. On
//! le signale ([`Bilan::version`] à `None`) sans faire échouer la mise à
//! jour : refuser une base valide parce qu'un second serveur n'a pas répondu
//! coûterait une initialisation entière pour rien.

use std::path::Path;

use ygo_core::paths::Paths;
use ygo_core::version::{self, InfoVersion};
use ygo_db::init::Comptages;
use ygo_sources::ClientHttp;

use crate::error::Result;
use crate::init;

/// Ce que l'application sait de sa base, sans toucher au réseau.
///
/// C'est ce qu'un écran de réglages a besoin d'afficher pour que le bouton
/// « Mettre à jour » veuille dire quelque chose : sans la version ni la date,
/// l'utilisateur ne peut pas juger s'il doit cliquer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EtatLocal {
    /// `bdd/cardinfo.db` existe et n'est pas vide.
    pub base_presente: bool,
    /// Taille de `bdd/cardinfo.db`, en octets.
    pub octets: Option<u64>,
    /// Version retenue dans `bdd/last_update.txt`, p. ex. `"146.68"`.
    pub version: Option<String>,
    /// Horodatage retenu, p. ex. `"2026-08-21 00:00:28"`.
    pub horodatage: Option<String>,
}

impl EtatLocal {
    /// Une phrase, telle qu'on peut la montrer sous un bouton.
    ///
    /// Elle dit ce qui est vrai, jamais plus : une base présente sans fichier
    /// de version n'est pas « à jour », elle est « de version inconnue ».
    #[must_use]
    pub fn resume(&self) -> String {
        if !self.base_presente {
            return "Base absente — la mise à jour la construira.".to_owned();
        }
        let taille = self.octets.map_or_else(String::new, |o| {
            #[allow(clippy::cast_precision_loss)]
            let mo = o as f64 / (1024.0 * 1024.0);
            format!(" — {mo:.0} Mo")
        });
        match (&self.version, &self.horodatage) {
            (Some(v), Some(h)) => format!("Version {v}, du {h}{taille}"),
            (Some(v), None) => format!("Version {v}{taille}"),
            (None, _) => format!("Version inconnue — `last_update.txt` absent{taille}"),
        }
    }
}

/// Lit l'état local de la base.
///
/// Ne touche ni au réseau ni à SQLite : deux `stat` et une lecture de JSON.
/// Un écran peut donc l'appeler à l'ouverture sans se figer.
#[must_use]
pub fn etat_local(paths: &Paths) -> EtatLocal {
    let base = paths.cardinfo_db();
    let octets = std::fs::metadata(&base).ok().map(|m| m.len());
    let infos = version::charger(paths.last_update());
    let premiere = infos.as_deref().and_then(<[InfoVersion]>::first);
    EtatLocal {
        base_presente: octets.is_some_and(|o| o > 0),
        octets,
        version: premiere.map(|i| i.database_version.clone()),
        horodatage: premiere.map(|i| i.last_update.clone()),
    }
}

/// Ce que le contrôle de version conclut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// La version locale est celle du serveur.
    AJour {
        /// La version commune.
        version: String,
    },
    /// Il y a quelque chose à faire — version différente, ou base absente.
    AFaire {
        /// Ce que l'on a, si on a quelque chose.
        locale: Option<String>,
        /// Ce que le serveur annonce.
        distante: String,
    },
    /// Le serveur n'a pas répondu — on ne conclut rien.
    Indecidable {
        /// Ce qui a empêché de conclure.
        raison: String,
    },
}

/// Tranche entre l'état local et la réponse du serveur.
///
/// Fonction **pure**, séparée de [`verifier`] pour être éprouvée sans réseau.
/// La comparaison est celle du Python : une inégalité de chaînes, pas un
/// ordre — `"146.7"` et `"146.68"` sont différentes sans que l'une soit
/// « plus récente » (cf. [`ygo_core::version::mise_a_jour_necessaire`]).
#[must_use]
pub fn decider(etat: &EtatLocal, distante: &str) -> Verdict {
    // Une base absente est à refaire quoi qu'en dise le fichier de version :
    // le `last_update.txt` d'une base supprimée à la main mentirait sinon.
    if !etat.base_presente {
        return Verdict::AFaire {
            locale: etat.version.clone(),
            distante: distante.to_owned(),
        };
    }
    let locale = etat.version.as_deref();
    if version::mise_a_jour_necessaire(locale, distante) {
        Verdict::AFaire {
            locale: etat.version.clone(),
            distante: distante.to_owned(),
        }
    } else {
        Verdict::AJour {
            version: distante.to_owned(),
        }
    }
}

/// Interroge le serveur et tranche.
///
/// Un échec réseau n'est pas une erreur remontée : c'est un
/// [`Verdict::Indecidable`]. Cet appel sert à décorer un bouton, pas à
/// autoriser une action — l'utilisateur doit pouvoir lancer la mise à jour
/// même si le contrôle de version a échoué.
pub async fn verifier(paths: &Paths, client: &ClientHttp) -> Verdict {
    let etat = etat_local(paths);
    let verdict = interroger(&etat, client).await;
    match &verdict {
        Verdict::AJour { version } => tracing::info!(version = %version, "base à jour"),
        Verdict::AFaire { locale, distante } => tracing::info!(
            locale = locale.as_deref().unwrap_or("aucune"),
            distante = %distante,
            "mise à jour de la base disponible"
        ),
        Verdict::Indecidable { raison } => {
            tracing::warn!(raison = %raison, "version distante indisponible");
        }
    }
    verdict
}

/// L'appel seul, sans journal — c'est [`verifier`] qui journalise.
async fn interroger(etat: &EtatLocal, client: &ClientHttp) -> Verdict {
    match ygo_sources::version::version_distante(client).await {
        Ok(infos) => match infos.first() {
            Some(info) => decider(etat, &info.database_version),
            None => Verdict::Indecidable {
                raison: "le serveur a répondu une liste vide".to_owned(),
            },
        },
        Err(e) => Verdict::Indecidable {
            raison: e.to_string(),
        },
    }
}

/// Ce que la mise à jour a produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bilan {
    /// Les comptages de l'initialisation.
    pub comptages: Comptages,
    /// La version enregistrée — `None` si le contrôle de version a échoué.
    pub version: Option<String>,
}

/// Reconstruit la base, puis enregistre la version obtenue.
///
/// L'enchaînement complet de « MAJ BDD » :
///
/// 1. [`init::initialiser`] — archive, parsing, écriture, vérification ;
/// 2. `checkDBVer.php` — la version que l'on vient d'obtenir ;
/// 3. `bdd/last_update.txt` — pour que l'étape 2 du prochain démarrage sache.
///
/// La progression est celle de [`crate::init`] : ce module n'en ajoute pas.
/// L'étape 3 dure quelques millisecondes, elle n'a rien à annoncer.
///
/// # Erreurs
///
/// Remonte l'échec de l'initialisation. L'échec des étapes 2 et 3 est
/// journalisé et laisse [`Bilan::version`] à `None` : la base, elle, est
/// bonne.
pub async fn mettre_a_jour(
    paths: &Paths,
    client: &ClientHttp,
    options: &init::Options,
    progression: Option<&init::Progression>,
) -> Result<Bilan> {
    // Les bornes du journal : sans elles, on lit une suite d'étapes sans
    // savoir laquelle appartient à quelle tentative.
    tracing::info!(installation = %paths.racine().display(), "MAJ BDD — début");
    // L'échec n'est pas journalisé ici : `init` nomme déjà l'étape fautive,
    // ce qui est plus utile qu'un second message qui redirait l'erreur sans
    // dire où.
    let comptages = init::initialiser(paths, options, progression).await?;
    let version = enregistrer_version(paths, client).await;
    // Une nouvelle base peut porter de nouvelles adresses d'image : c'est le
    // moment de retenter ce que la source primaire avait refusé (demande du
    // 2026-10-02). Rien ne part ici — les images suivent à l'ouverture.
    match crate::replis::reprendre(paths, client).await {
        Ok(r) => tracing::info!(
            classeurs = r.classeurs,
            adresses_reposees = r.adresses_reposees,
            replis = r.replis,
            verifications = r.verifications,
            presents = r.presents,
            absents = r.absents,
            effaces = r.effaces,
            "images : reprise d'après mise à jour"
        ),
        Err(e) => tracing::warn!(erreur = %e, "images : reprise d'après mise à jour incomplète"),
    }
    // Les noms FR officiels que la nouvelle base apporte — jamais inventés,
    // jamais effacés (R8, 2026-10-02).
    match crate::noms_fr::rafraichir(paths) {
        Ok(b) => tracing::info!(
            classeurs = b.classeurs,
            ajoutes = b.ajoutes,
            corriges = b.corriges,
            sans_source = b.sans_source,
            "noms FR : rafraîchis d'après la base"
        ),
        Err(e) => tracing::warn!(erreur = %e, "noms FR non rafraîchis"),
    }
    tracing::info!(
        version = version.as_deref().unwrap_or("inconnue"),
        cartes = comptages.cartes,
        tirages = comptages.tirages,
        "MAJ BDD — terminée"
    );
    Ok(Bilan { comptages, version })
}

/// Écrit `bdd/last_update.txt` d'après le serveur. Silencieux en cas d'échec.
async fn enregistrer_version(paths: &Paths, client: &ClientHttp) -> Option<String> {
    let infos = match ygo_sources::version::version_distante(client).await {
        Ok(infos) => infos,
        Err(e) => {
            tracing::warn!(erreur = %e, "version distante indisponible — last_update.txt non écrit");
            return None;
        }
    };
    let premiere = infos.first()?.database_version.clone();
    if let Err(e) = ecrire_version(paths.last_update(), &infos) {
        tracing::warn!(erreur = %e, "last_update.txt non écrit");
        return None;
    }
    tracing::info!(version = %premiere, "version enregistrée dans last_update.txt");
    Some(premiere)
}

/// L'écriture seule, pour qu'un test puisse la jouer sans réseau.
fn ecrire_version(chemin: impl AsRef<Path>, infos: &[InfoVersion]) -> Result<()> {
    version::enregistrer(chemin, infos)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn info(v: &str) -> InfoVersion {
        InfoVersion {
            database_version: v.to_owned(),
            last_update: "2026-08-21 00:00:28".to_owned(),
        }
    }

    /// Le cas qui motive tout le module : une installation neuve ne sait rien.
    #[test]
    fn une_installation_vide_n_a_ni_base_ni_version() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(temp.path());
        let etat = etat_local(&paths);
        assert!(!etat.base_presente);
        assert_eq!(etat.version, None);
        assert_eq!(etat.octets, None);
        assert!(
            etat.resume().contains("absente"),
            "et elle le dit : {}",
            etat.resume()
        );
    }

    #[test]
    fn une_base_presente_avec_sa_version_se_resume_en_une_phrase() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(temp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"pas une vraie base, mais non vide").unwrap();
        version::enregistrer(paths.last_update(), &[info("146.68")]).unwrap();

        let etat = etat_local(&paths);
        assert!(etat.base_presente);
        assert_eq!(etat.version.as_deref(), Some("146.68"));
        assert_eq!(etat.horodatage.as_deref(), Some("2026-08-21 00:00:28"));
        let resume = etat.resume();
        assert!(resume.contains("146.68"), "{resume}");
        assert!(resume.contains("2026-08-21"), "{resume}");
    }

    /// Une base de zéro octet n'est pas une base : `run_init` interrompu en
    /// laisse une, et la déclarer présente empêcherait de la refaire.
    #[test]
    fn une_base_vide_ne_compte_pas_comme_presente() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(temp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"").unwrap();
        assert!(!etat_local(&paths).base_presente);
    }

    #[test]
    fn meme_version_et_base_presente_donnent_a_jour() {
        let etat = EtatLocal {
            base_presente: true,
            octets: Some(170_000_000),
            version: Some("146.68".to_owned()),
            horodatage: Some("2026-08-21 00:00:28".to_owned()),
        };
        assert_eq!(
            decider(&etat, "146.68"),
            Verdict::AJour {
                version: "146.68".to_owned()
            }
        );
    }

    #[test]
    fn version_differente_donne_a_faire() {
        let etat = EtatLocal {
            base_presente: true,
            octets: Some(170_000_000),
            version: Some("146.68".to_owned()),
            horodatage: None,
        };
        assert_eq!(
            decider(&etat, "147.01"),
            Verdict::AFaire {
                locale: Some("146.68".to_owned()),
                distante: "147.01".to_owned()
            }
        );
    }

    /// Le piège : un `last_update.txt` resté là après une suppression de la
    /// base à la main. La version dit « à jour », le disque dit « rien ».
    #[test]
    fn une_version_sans_base_reste_a_faire() {
        let etat = EtatLocal {
            base_presente: false,
            octets: None,
            version: Some("146.68".to_owned()),
            horodatage: None,
        };
        assert_eq!(
            decider(&etat, "146.68"),
            Verdict::AFaire {
                locale: Some("146.68".to_owned()),
                distante: "146.68".to_owned()
            },
            "la base absente l'emporte sur le fichier de version"
        );
    }

    /// La comparaison est textuelle, comme en Python : `146.7` ≠ `146.68`,
    /// sans que l'une soit déclarée plus récente que l'autre.
    #[test]
    fn la_comparaison_reste_textuelle() {
        let etat = EtatLocal {
            base_presente: true,
            octets: Some(1),
            version: Some("146.7".to_owned()),
            horodatage: None,
        };
        assert!(matches!(decider(&etat, "146.68"), Verdict::AFaire { .. }));
    }

    /// L'aller-retour disque : ce que l'on écrit après une mise à jour est ce
    /// que l'on relit au démarrage suivant. C'est tout l'objet du module.
    #[test]
    fn la_version_ecrite_est_celle_qui_se_relit() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(temp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"base").unwrap();

        assert!(matches!(
            decider(&etat_local(&paths), "146.68"),
            Verdict::AFaire { locale: None, .. }
        ));

        ecrire_version(paths.last_update(), &[info("146.68")]).unwrap();

        assert_eq!(
            decider(&etat_local(&paths), "146.68"),
            Verdict::AJour {
                version: "146.68".to_owned()
            },
            "sans cette écriture, la base se déclarerait périmée aussitôt construite"
        );
    }
}
