// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Cardmarket — les fichiers publics du catalogue et du guide des prix.
//!
//! # Trois fichiers, une fois par jour, sans compte
//!
//! Cardmarket met à disposition, pour chaque jeu, son catalogue et son guide
//! des prix sous forme de fichiers JSON publics, régénérés chaque nuit
//! (Yu-Gi-Oh! = jeu `3`) :
//!
//! | Fichier | Contenu | Taille (oct. 2026) |
//! |---|---|---|
//! | `products_singles_3.json` | 87 393 cartes : `idProduct`, nom anglais, `idExpansion`, date d'ajout | 15 Mo |
//! | `products_nonsingles_3.json` | produits scellés — leurs noms disent celui des extensions | 0,4 Mo |
//! | `price_guide_3.json` | prix par `idProduct` : tendance, plus bas, moyennes | 18 Mo |
//!
//! Aucune rareté, aucun numéro de carte : c'est `ygo_core::cardmarket` qui
//! les reconstitue.
//!
//! # Les règles qu'on s'impose
//!
//! L'application est partagée ; la règle vaut pour cent utilisateurs comme
//! pour un :
//!
//! - **une vérification par fichier et par jour au plus** ([`INTERVALLE`]),
//!   puisque les fichiers ne changent qu'une fois par jour ;
//! - **requête conditionnelle** : l'`ETag` et la date reçus la dernière fois
//!   sont renvoyés (`If-None-Match`, `If-Modified-Since`) ; un fichier
//!   inchangé coûte une réponse `304` vide, pas 18 Mo ;
//! - **rien vers `www.cardmarket.com`** : le site est derrière une protection
//!   anti-robots, et l'application n'a rien à y faire. Elle n'en construit
//!   que des adresses, que l'utilisateur ouvre lui-même.
//!
//! # Écriture
//!
//! Chaque fichier descend dans un temporaire du même dossier, puis remplace
//! l'ancien d'un renommage : une coupure en plein téléchargement laisse la
//! version de la veille intacte.

use std::collections::BTreeMap;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SourceError};
use crate::http::{
    ClientHttp, BACKOFF_BASE, DELAI_TELECHARGEMENT, STATUTS_A_REESSAYER, TENTATIVES,
};

/// Racine des fichiers publics.
pub const BASE: &str = "https://downloads.s3.cardmarket.com/productCatalog";

/// Une vérification par fichier et par jour, au plus.
pub const INTERVALLE: Duration = Duration::from_secs(24 * 3600);

/// Le registre des vérifications, à côté des fichiers.
pub const FICHIER_ETAT: &str = "etat.json";

/// Les trois fichiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fichier {
    /// Les cartes à l'unité.
    Cartes,
    /// Les produits scellés.
    Scelles,
    /// Le guide des prix.
    Prix,
}

impl Fichier {
    /// Les trois, dans l'ordre où ils se téléchargent.
    pub const TOUS: [Self; 3] = [Self::Cartes, Self::Scelles, Self::Prix];

    /// Nom du fichier — le même ici et sur le serveur.
    #[must_use]
    pub fn nom(self) -> &'static str {
        match self {
            Self::Cartes => "products_singles_3.json",
            Self::Scelles => "products_nonsingles_3.json",
            Self::Prix => "price_guide_3.json",
        }
    }

    /// Adresse publique.
    ///
    /// ```
    /// use ygo_sources::cardmarket::Fichier;
    /// assert_eq!(
    ///     Fichier::Prix.url(),
    ///     "https://downloads.s3.cardmarket.com/productCatalog/priceGuide/price_guide_3.json"
    /// );
    /// ```
    #[must_use]
    pub fn url(self) -> String {
        let dossier = match self {
            Self::Cartes | Self::Scelles => "productList",
            Self::Prix => "priceGuide",
        };
        format!("{BASE}/{dossier}/{}", self.nom())
    }
}

/// Ce qu'on retient d'un fichier entre deux vérifications.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtatFichier {
    /// L'`ETag` de la dernière réponse complète.
    #[serde(default)]
    pub etag: Option<String>,
    /// Son en-tête `Last-Modified`.
    #[serde(default)]
    pub modifie: Option<String>,
    /// Dernière vérification aboutie (secondes Unix) — `304` compris.
    #[serde(default)]
    pub verifie_le: i64,
    /// Dernier téléchargement complet (secondes Unix).
    #[serde(default)]
    pub recu_le: i64,
}

/// Le registre `etat.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Etat {
    /// Par nom de fichier.
    #[serde(default)]
    pub fichiers: BTreeMap<String, EtatFichier>,
}

impl Etat {
    /// Relit le registre ; vide s'il manque ou ne se lit pas — tout sera
    /// alors vérifié, ce qui n'est qu'un peu de réseau.
    #[must_use]
    pub fn charger(dossier: &Path) -> Self {
        std::fs::read(dossier.join(FICHIER_ETAT))
            .ok()
            .and_then(|o| serde_json::from_slice(&o).ok())
            .unwrap_or_default()
    }

    /// Écrit le registre, d'un renommage.
    ///
    /// # Errors
    ///
    /// Si le dossier n'accepte pas l'écriture.
    pub fn enregistrer(&self, dossier: &Path) -> Result<()> {
        let octets = serde_json::to_vec_pretty(self)
            .map_err(|e| SourceError::deserialisation(FICHIER_ETAT, e))?;
        ecrire_atomiquement(&dossier.join(FICHIER_ETAT), &octets)
    }

    /// L'état d'un fichier.
    #[must_use]
    pub fn de(&self, fichier: Fichier) -> Option<&EtatFichier> {
        self.fichiers.get(fichier.nom())
    }

    /// Faut-il redemander ce fichier ?
    ///
    /// Oui s'il manque sur le disque, ou si la dernière vérification aboutie
    /// date d'au moins [`INTERVALLE`] — ou est **dans le futur**, horloge
    /// remise à l'heure : on ne reste pas bloqué jusqu'à cette date.
    #[must_use]
    pub fn a_verifier(&self, fichier: Fichier, dossier: &Path, maintenant: i64) -> bool {
        if !dossier.join(fichier.nom()).is_file() {
            return true;
        }
        let Some(e) = self.de(fichier) else {
            return true;
        };
        let ecoule = maintenant - e.verifie_le;
        ecoule < 0 || ecoule >= i64::try_from(INTERVALLE.as_secs()).unwrap_or(i64::MAX)
    }
}

/// Ce qu'une vérification a donné pour un fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// Vérifié il y a moins d'un jour : pas redemandé.
    PasEncore,
    /// Le serveur a répondu `304` : notre copie est la bonne.
    Inchange,
    /// Une version nouvelle est arrivée.
    Recu {
        /// Octets écrits.
        octets: u64,
    },
    /// La vérification a échoué ; la copie précédente, s'il y en a une, reste.
    Echec(String),
}

/// Le bilan d'une vérification des trois fichiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Une issue par fichier.
    pub fichiers: Vec<(Fichier, Issue)>,
}

impl Bilan {
    /// Au moins un fichier a changé ?
    #[must_use]
    pub fn a_change(&self) -> bool {
        self.fichiers
            .iter()
            .any(|(_, i)| matches!(i, Issue::Recu { .. }))
    }

    /// Les échecs, en clair.
    #[must_use]
    pub fn echecs(&self) -> Vec<String> {
        self.fichiers
            .iter()
            .filter_map(|(f, i)| match i {
                Issue::Echec(e) => Some(format!("{} : {e}", f.nom())),
                _ => None,
            })
            .collect()
    }
}

/// L'heure Unix, en secondes.
#[must_use]
pub fn maintenant() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Vérifie les trois fichiers et télécharge ceux qui ont changé.
///
/// `forcer` passe outre l'intervalle d'un jour — jamais la requête
/// conditionnelle : un fichier inchangé reste une réponse `304`.
///
/// Un fichier en échec n'arrête pas les autres ; son issue le dit.
///
/// # Errors
///
/// Seulement si le dossier ne peut pas être créé.
pub async fn telecharger(client: &ClientHttp, dossier: &Path, forcer: bool) -> Result<Bilan> {
    telecharger_depuis(client, dossier, forcer, BASE).await
}

/// [`telecharger`], depuis une autre racine — pour les tests.
///
/// # Errors
///
/// Comme [`telecharger`].
pub async fn telecharger_depuis(
    client: &ClientHttp,
    dossier: &Path,
    forcer: bool,
    base: &str,
) -> Result<Bilan> {
    std::fs::create_dir_all(dossier).map_err(|e| SourceError::io(dossier, e))?;
    let mut etat = Etat::charger(dossier);
    let mut bilan = Bilan::default();
    for fichier in Fichier::TOUS {
        let instant = maintenant();
        if !forcer && !etat.a_verifier(fichier, dossier, instant) {
            bilan.fichiers.push((fichier, Issue::PasEncore));
            continue;
        }
        let url = fichier.url().replacen(BASE, base, 1);
        let present = dossier.join(fichier.nom()).is_file();
        let precedent = etat.de(fichier).cloned().filter(|_| present);
        let issue = match telecharger_un(client, &url, dossier, fichier, precedent.as_ref()).await {
            Ok(Reponse::NonModifie) => {
                let e = etat.fichiers.entry(fichier.nom().to_owned()).or_default();
                e.verifie_le = instant;
                Issue::Inchange
            }
            Ok(Reponse::Recu {
                etag,
                modifie,
                octets,
            }) => {
                let e = etat.fichiers.entry(fichier.nom().to_owned()).or_default();
                e.etag = etag;
                e.modifie = modifie;
                e.verifie_le = instant;
                e.recu_le = instant;
                Issue::Recu { octets }
            }
            Err(e) => {
                tracing::warn!(fichier = fichier.nom(), erreur = %e, "Cardmarket : fichier non vérifié");
                Issue::Echec(e.to_string())
            }
        };
        bilan.fichiers.push((fichier, issue));
        // Retenu au fil de l'eau : une coupure entre deux fichiers ne fait
        // pas redemander le premier.
        if let Err(e) = etat.enregistrer(dossier) {
            tracing::warn!(erreur = %e, "Cardmarket : registre non écrit");
        }
    }
    Ok(bilan)
}

/// La réponse du serveur à une requête conditionnelle.
enum Reponse {
    NonModifie,
    Recu {
        etag: Option<String>,
        modifie: Option<String>,
        octets: u64,
    },
}

/// Une requête conditionnelle, avec tentatives sur erreur transitoire.
async fn telecharger_un(
    client: &ClientHttp,
    url: &str,
    dossier: &Path,
    fichier: Fichier,
    precedent: Option<&EtatFichier>,
) -> Result<Reponse> {
    let mut derniere: Option<SourceError> = None;
    for tentative in 0..TENTATIVES {
        if tentative > 0 {
            tokio::time::sleep(BACKOFF_BASE * 2_u32.pow(tentative - 1)).await;
        }
        // `brut()` : le client du programme, pour un appelant qui streame
        // lui-même. Aucun quota ne vise cet hôte — trois requêtes par jour.
        let mut requete = client.brut().get(url).timeout(DELAI_TELECHARGEMENT);
        if let Some(p) = precedent {
            if let Some(etag) = &p.etag {
                requete = requete.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(date) = &p.modifie {
                requete = requete.header(reqwest::header::IF_MODIFIED_SINCE, date);
            }
        }
        let reponse = match requete.send().await {
            Ok(r) => r,
            Err(e) => {
                derniere = Some(SourceError::reseau(url, e));
                continue;
            }
        };
        let statut = reponse.status().as_u16();
        if statut == 304 {
            return Ok(Reponse::NonModifie);
        }
        if STATUTS_A_REESSAYER.contains(&statut) {
            derniere = Some(SourceError::Statut {
                url: url.to_owned(),
                statut,
            });
            continue;
        }
        if !reponse.status().is_success() {
            return Err(SourceError::Statut {
                url: url.to_owned(),
                statut,
            });
        }
        let entete = |nom: reqwest::header::HeaderName| {
            reponse
                .headers()
                .get(nom)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let etag = entete(reqwest::header::ETAG);
        let modifie = entete(reqwest::header::LAST_MODIFIED);
        match ecrire_corps(reponse, url, dossier, fichier).await {
            Ok(octets) => {
                return Ok(Reponse::Recu {
                    etag,
                    modifie,
                    octets,
                })
            }
            Err(e @ SourceError::Reseau { .. }) => derniere = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(derniere.unwrap_or_else(|| SourceError::Statut {
        url: url.to_owned(),
        statut: 0,
    }))
}

/// Écrit le corps dans un temporaire, vérifie que c'est du JSON, puis
/// remplace le fichier.
async fn ecrire_corps(
    reponse: reqwest::Response,
    url: &str,
    dossier: &Path,
    fichier: Fichier,
) -> Result<u64> {
    let mut tmp =
        tempfile::NamedTempFile::new_in(dossier).map_err(|e| SourceError::io(dossier, e))?;
    let mut octets: u64 = 0;
    let mut flux = reponse.bytes_stream();
    while let Some(morceau) = flux.next().await {
        let morceau = morceau.map_err(|e| SourceError::reseau(url, e))?;
        tmp.write_all(&morceau)
            .map_err(|e| SourceError::io(tmp.path(), e))?;
        octets += u64::try_from(morceau.len()).unwrap_or(0);
    }
    tmp.flush().map_err(|e| SourceError::io(tmp.path(), e))?;
    // Une page d'erreur servie en 200 ne doit pas remplacer un bon fichier.
    if !commence_par_un_objet(tmp.path()) {
        return Err(SourceError::Archive(format!(
            "{} : le contenu reçu n'est pas un objet JSON",
            fichier.nom()
        )));
    }
    let cible = dossier.join(fichier.nom());
    tmp.persist(&cible)
        .map_err(|e| SourceError::io(&cible, e.error))?;
    Ok(octets)
}

/// Le fichier commence-t-il par `{`, espaces mis à part ?
fn commence_par_un_objet(chemin: &Path) -> bool {
    let Ok(f) = std::fs::File::open(chemin) else {
        return false;
    };
    let mut debut = Vec::with_capacity(64);
    if f.take(64).read_to_end(&mut debut).is_err() {
        return false;
    }
    debut
        .iter()
        .find(|o| !o.is_ascii_whitespace())
        .is_some_and(|o| *o == b'{')
}

/// Écrit un petit fichier d'un renommage.
fn ecrire_atomiquement(cible: &Path, octets: &[u8]) -> Result<()> {
    let dossier = cible.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp =
        tempfile::NamedTempFile::new_in(dossier).map_err(|e| SourceError::io(dossier, e))?;
    tmp.write_all(octets)
        .map_err(|e| SourceError::io(tmp.path(), e))?;
    tmp.persist(cible)
        .map_err(|e| SourceError::io(cible, e.error))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Lecture
// ─────────────────────────────────────────────────────────────────────────────

/// Un produit du catalogue — carte ou produit scellé.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Produit {
    /// Identifiant Cardmarket.
    #[serde(rename = "idProduct")]
    pub id: i64,
    /// Nom anglais, tel que Cardmarket l'écrit (`""A"" Cell Breeding Device`).
    pub name: String,
    /// L'extension.
    #[serde(rename = "idExpansion", default)]
    pub id_extension: i64,
    /// Date de création du produit — `2007-01-01 00:00:00` pour les plus
    /// anciens, faute de mieux chez Cardmarket.
    #[serde(rename = "dateAdded", default)]
    pub ajoute_le: String,
}

/// Un fichier de produits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Catalogue {
    /// Date de génération du fichier.
    #[serde(rename = "createdAt", default)]
    pub cree_le: String,
    /// Les produits.
    #[serde(default)]
    pub products: Vec<Produit>,
}

/// Les prix d'un produit, en euros, tels que publiés.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct PrixPublie {
    /// Identifiant du produit.
    #[serde(rename = "idProduct")]
    pub id: i64,
    /// Tendance.
    #[serde(default)]
    pub trend: Option<f64>,
    /// Plus bas prix proposé.
    #[serde(default)]
    pub low: Option<f64>,
    /// Prix de vente moyen.
    #[serde(default)]
    pub avg: Option<f64>,
    /// Moyenne sur un jour.
    #[serde(default)]
    pub avg1: Option<f64>,
    /// Moyenne sur sept jours.
    #[serde(default)]
    pub avg7: Option<f64>,
    /// Moyenne sur trente jours.
    #[serde(default)]
    pub avg30: Option<f64>,
}

/// Le guide des prix.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GuidePrix {
    /// Date de génération du fichier.
    #[serde(rename = "createdAt", default)]
    pub cree_le: String,
    /// Les prix, un par produit.
    #[serde(rename = "priceGuides", default)]
    pub prix: Vec<PrixPublie>,
}

/// Lit un fichier de produits (cartes ou scellés).
///
/// # Errors
///
/// Fichier absent ou illisible.
pub fn lire_catalogue(chemin: &Path) -> Result<Catalogue> {
    lire_json(chemin)
}

/// Lit le guide des prix.
///
/// # Errors
///
/// Fichier absent ou illisible.
pub fn lire_prix(chemin: &Path) -> Result<GuidePrix> {
    lire_json(chemin)
}

fn lire_json<T: serde::de::DeserializeOwned>(chemin: &Path) -> Result<T> {
    let f = std::fs::File::open(chemin).map_err(|e| SourceError::io(chemin, e))?;
    serde_json::from_reader(BufReader::with_capacity(1 << 20, f))
        .map_err(|e| SourceError::deserialisation(chemin.display().to_string(), e))
}

/// La date de génération d'un fichier, lue dans ses premiers octets sans
/// le parser en entier — de quoi savoir si l'index est à refaire.
///
/// ```
/// let dir = tempfile::tempdir().unwrap();
/// let f = dir.path().join("p.json");
/// std::fs::write(&f, r#"{"version":1,"createdAt":"2026-10-20T02:44:58+0200","priceGuides":[]}"#).unwrap();
/// assert_eq!(ygo_sources::cardmarket::date_du_fichier(&f).as_deref(), Some("2026-10-20T02:44:58+0200"));
/// ```
#[must_use]
pub fn date_du_fichier(chemin: &Path) -> Option<String> {
    let f = std::fs::File::open(chemin).ok()?;
    let mut debut = Vec::with_capacity(256);
    f.take(256).read_to_end(&mut debut).ok()?;
    let texte = String::from_utf8_lossy(&debut);
    let apres = texte.split_once("\"createdAt\"")?.1;
    let valeur = apres.split_once('"')?.1;
    Some(valeur.split_once('"')?.0.to_owned())
}

/// Où se trouvent les fichiers d'un dossier, nommés.
#[must_use]
pub fn chemin(dossier: &Path, fichier: Fichier) -> PathBuf {
    dossier.join(fichier.nom())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use super::*;

    #[test]
    fn les_fichiers_se_lisent_sous_leur_forme_reelle() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.json");
        std::fs::write(
            &p,
            r#"{"version":1,"createdAt":"2026-10-20T14:24:50+0200","products":[
            {"idProduct":101788,"name":"\"\"A\"\" Cell Breeding Device","idCategory":5,
             "categoryName":"Yugioh Single","idExpansion":1011,"idMetacard":101779,
             "dateAdded":"2007-01-01 00:00:00"}]}"#,
        )
        .unwrap();
        let c = lire_catalogue(&p).unwrap();
        assert_eq!(c.cree_le, "2026-10-20T14:24:50+0200");
        assert_eq!(c.products[0].id, 101_788);
        assert_eq!(c.products[0].id_extension, 1011);
        assert_eq!(c.products[0].name, "\"\"A\"\" Cell Breeding Device");

        let g = dir.path().join("g.json");
        std::fs::write(
            &g,
            r#"{"version":1,"createdAt":"2026-10-20T02:44:58+0200","priceGuides":[
            {"idProduct":101788,"idCategory":5,"avg":0.08,"low":0.02,"trend":0.08,"avg1":0.2,
             "avg7":0.09,"avg30":0.12,"avg-foil":null,"low-foil":null,"trend-foil":0.1}]}"#,
        )
        .unwrap();
        let prix = lire_prix(&g).unwrap();
        assert_eq!(prix.prix[0].trend, Some(0.08));
        assert_eq!(prix.prix[0].avg30, Some(0.12));
    }

    #[test]
    fn un_fichier_absent_ou_vieux_d_un_jour_est_a_verifier() {
        let dir = tempfile::tempdir().unwrap();
        let mut etat = Etat::default();
        assert!(etat.a_verifier(Fichier::Prix, dir.path(), 1_000_000));
        std::fs::write(dir.path().join(Fichier::Prix.nom()), "{}").unwrap();
        assert!(
            etat.a_verifier(Fichier::Prix, dir.path(), 1_000_000),
            "jamais vérifié"
        );
        etat.fichiers.insert(
            Fichier::Prix.nom().to_owned(),
            EtatFichier {
                verifie_le: 1_000_000,
                ..EtatFichier::default()
            },
        );
        assert!(!etat.a_verifier(Fichier::Prix, dir.path(), 1_000_000 + 3600));
        assert!(etat.a_verifier(Fichier::Prix, dir.path(), 1_000_000 + 86_400));
        assert!(
            etat.a_verifier(Fichier::Prix, dir.path(), 999_000),
            "horloge reculée"
        );
    }

    #[test]
    fn le_registre_survit_a_un_redemarrage() {
        let dir = tempfile::tempdir().unwrap();
        let mut etat = Etat::default();
        etat.fichiers.insert(
            "x".into(),
            EtatFichier {
                etag: Some("\"abc\"".into()),
                verifie_le: 5,
                ..EtatFichier::default()
            },
        );
        etat.enregistrer(dir.path()).unwrap();
        assert_eq!(Etat::charger(dir.path()), etat);
    }

    #[test]
    fn un_objet_json_se_reconnait() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok");
        std::fs::write(&ok, "  \n{\"a\":1}").unwrap();
        assert!(commence_par_un_objet(&ok));
        let html = dir.path().join("html");
        std::fs::write(&html, "<!DOCTYPE html>").unwrap();
        assert!(!commence_par_un_objet(&html));
    }
}
