// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La mise à jour **de l'application** depuis les Releases GitHub.
//!
//! À ne pas confondre avec [`crate::maj`], qui reconstruit la base de
//! référence.
//!
//! # Le parcours
//!
//! 1. [`verifier`] — une requête à GitHub ; la Release la plus récente plus
//!    récente que l'exécutable est retenue ([`choisir`]). Les versions de test
//!    (« pre-release ») ne sont proposées que si l'utilisateur l'a demandé.
//! 2. L'interface propose ; **rien ne se télécharge sans clic**.
//! 3. [`telecharger`] — l'exécutable joint à la Release, vérifié deux fois :
//!    - son **empreinte SHA-256** : celle que GitHub calcule lui-même à la
//!      publication (champ `digest` de l'API), à défaut celle d'un
//!      `ygo-ui.exe.sha256` joint à côté ;
//!    - sa **marque de version** ([`MARQUE`]) : l'exécutable doit porter le
//!      numéro du tag. Un tag `v2.0.1` sur un exe compilé en `2.0.0-alpha.1`
//!      ferait redescendre l'utilisateur, puis lui reproposer la « mise à
//!      jour » à chaque démarrage.
//!
//!    Un fichier qui échoue à l'un ou l'autre est effacé : il ne remplace
//!    jamais l'exécutable.
//! 4. [`sauvegarder`] — une copie des classeurs et des réglages, au cas où.
//! 5. [`installer`] — Windows interdit d'écrire sur un exécutable en cours
//!    d'exécution, mais pas de le **renommer** : l'actuel devient
//!    `ygo-ui.exe.ancien`, le nouveau prend sa place, l'application redémarre.
//!    [`nettoyer`] efface l'ancien au lancement suivant.
//!
//! # Ce qui n'est jamais touché
//!
//! Les données : classeurs, images, base. Seul l'exécutable change. Une
//! version qui ferait évoluer la structure des classeurs les met à niveau
//! elle-même, comme aujourd'hui (`ygo_db::migrations`).
//!
//! # Une Release sans empreinte
//!
//! Elle est signalée, avec un lien vers sa page, mais n'est pas installable
//! depuis l'application : sans empreinte, rien ne prouve que le fichier reçu
//! est celui qui a été publié. Depuis que GitHub calcule l'empreinte de chaque
//! fichier joint, ce cas ne vise plus que des Releases anciennes.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ygo_core::numero_version::NumeroVersion;
use ygo_core::paths::Paths;
use ygo_sources::github::Release;

use crate::error::{AppError, Result};

/// Le nom de l'exécutable joint à chaque Release.
pub const EXE: &str = "ygo-ui.exe";

/// Le nom de son empreinte, jointe à côté — facultative quand GitHub fournit
/// la sienne.
pub const EMPREINTE: &str = "ygo-ui.exe.sha256";

/// Ce qui précède le numéro dans la marque de version.
const PREFIXE_MARQUE: &str = "YGO-UI-VERSION=";

/// La marque de version que porte tout exécutable : `YGO-UI-VERSION=2.0.1;`.
///
/// Le numéro est celui du `Cargo.toml` au moment de la compilation. L'
/// interface s'en sert au démarrage (journal) : elle reste donc dans le
/// binaire, où [`telecharger`] la cherche.
pub const MARQUE: &str = concat!("YGO-UI-VERSION=", env!("CARGO_PKG_VERSION"), ";");

/// D'où vient l'empreinte attendue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Empreinte {
    /// Calculée par GitHub à la publication : connue sans requête.
    Publiee(String),
    /// À lire dans un `ygo-ui.exe.sha256` joint : son adresse.
    Fichier(String),
}

/// Une nouvelle version, telle que l'interface la propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disponible {
    /// Le numéro, sans `v` — `2.0.0-alpha.2`.
    pub version: String,
    /// Le tag GitHub — `v2.0.0-alpha.2`.
    pub tag: String,
    /// Le titre de la Release.
    pub titre: String,
    /// Les nouveautés, telles que la Release les décrit (Markdown).
    pub notes: String,
    /// La page de la Release.
    pub page: String,
    /// Version de test ?
    pub pre_version: bool,
    /// L'exécutable joint : adresse et taille.
    pub exe: Option<(String, u64)>,
    /// Son empreinte attendue.
    pub empreinte: Option<Empreinte>,
}

impl Disponible {
    /// Peut-elle être installée depuis l'application ? Il faut l'exécutable
    /// **et** une empreinte — celle de GitHub ou un `.sha256` joint.
    #[must_use]
    pub fn installable(&self) -> bool {
        self.exe.is_some() && self.empreinte.is_some()
    }
}

/// Ce que le contrôle conclut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Aucune version plus récente.
    AJour,
    /// Une version plus récente est publiée.
    Disponible(Disponible),
    /// Une version plus récente existe, mais l'utilisateur l'a ignorée.
    Ignoree(Disponible),
}

/// La Release à proposer — fonction **pure**.
///
/// La plus récente au sens de [`NumeroVersion`], strictement plus récente que
/// `courante`. Une Release dont le tag n'est pas un numéro est écartée — c'est
/// le cas des anciennes, nommées « Ajout Inventaire… ». Une pré-version n'est
/// retenue que si `avec_pre`.
#[must_use]
pub fn choisir(releases: &[Release], courante: &str, avec_pre: bool) -> Option<Disponible> {
    let courante = NumeroVersion::lire(courante)?;
    releases
        .iter()
        .filter(|r| !r.draft && (avec_pre || !r.prerelease))
        .filter_map(|r| NumeroVersion::lire(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| *v > courante)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| Disponible {
            version: r.tag_name.trim().trim_start_matches(['v', 'V']).to_owned(),
            tag: r.tag_name.clone(),
            titre: r.name.clone().unwrap_or_else(|| r.tag_name.clone()),
            notes: r.body.clone().unwrap_or_default(),
            page: r.html_url.clone(),
            pre_version: r.prerelease,
            exe: r
                .fichier(EXE)
                .map(|f| (f.browser_download_url.clone(), f.size)),
            empreinte: r
                .fichier(EXE)
                .and_then(|f| f.digest.as_deref())
                .and_then(lire_digest)
                .map(Empreinte::Publiee)
                .or_else(|| {
                    r.fichier(EMPREINTE)
                        .map(|f| Empreinte::Fichier(f.browser_download_url.clone()))
                }),
        })
}

/// L'empreinte telle que GitHub la donne — `sha256:<64 hexa>`. Un autre
/// algorithme n'est pas retenu.
///
/// ```
/// use ygo_app::maj_appli::lire_digest;
/// let h = "4f".repeat(32);
/// assert_eq!(lire_digest(&format!("sha256:{h}")), Some(h));
/// assert_eq!(lire_digest("sha512:abcd"), None);
/// ```
#[must_use]
pub fn lire_digest(digest: &str) -> Option<String> {
    lire_empreinte(digest.trim().strip_prefix("sha256:")?)
}

/// Les numéros de version marqués dans un exécutable (cf. [`MARQUE`]).
///
/// Plusieurs peuvent apparaître : le préfixe seul figure aussi dans le code
/// qui le cherche ; seules les occurrences suivies d'un numéro lisible et
/// d'un `;` comptent.
#[must_use]
pub fn versions_marquees(octets: &[u8]) -> Vec<NumeroVersion> {
    let prefixe = PREFIXE_MARQUE.as_bytes();
    let mut trouvees = Vec::new();
    let mut depart = 0;
    while let Some(pos) = octets
        .get(depart..)
        .and_then(|reste| reste.windows(prefixe.len()).position(|w| w == prefixe))
    {
        let debut = depart + pos + prefixe.len();
        let suite = octets.get(debut..).unwrap_or_default();
        let fenetre = suite.get(..64).unwrap_or(suite);
        if let Some(fin) = fenetre.iter().position(|&o| o == b';') {
            if let Some(v) = fenetre
                .get(..fin)
                .and_then(|n| std::str::from_utf8(n).ok())
                .and_then(NumeroVersion::lire)
            {
                trouvees.push(v);
            }
        }
        depart = debut;
    }
    trouvees
}

/// L'exécutable porte-t-il la version du tag ? Sinon, pourquoi.
fn controler_version(octets: &[u8], dispo: &Disponible) -> std::result::Result<(), String> {
    let Some(attendue) = NumeroVersion::lire(&dispo.tag) else {
        return Err(format!(
            "le tag {} n'est pas un numéro de version",
            dispo.tag
        ));
    };
    let trouvees = versions_marquees(octets);
    if trouvees.contains(&attendue) {
        return Ok(());
    }
    Err(match trouvees.first() {
        Some(v) => format!(
            "l'exécutable joint à {} a été compilé pour une autre version ({v}) — \
             Release mal préparée, fichier écarté",
            dispo.tag
        ),
        None => format!(
            "l'exécutable joint à {} ne porte pas de numéro de version (compilé \
             avant la 2.0.1 ?) — fichier écarté",
            dispo.tag
        ),
    })
}

/// Ce que l'application retient d'un contrôle à l'autre.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memoire {
    /// La version que l'utilisateur a choisi d'ignorer.
    #[serde(default)]
    pub ignoree: Option<String>,
}

fn chemin_memoire(paths: &Paths) -> PathBuf {
    paths.bdd().join("maj_appli.json")
}

/// Lit la mémoire ; absente ou illisible, elle est vide.
#[must_use]
pub fn memoire(paths: &Paths) -> Memoire {
    std::fs::read(chemin_memoire(paths))
        .ok()
        .and_then(|o| serde_json::from_slice(&o).ok())
        .unwrap_or_default()
}

/// Retient qu'une version est à ignorer : elle ne sera plus proposée, la
/// suivante si.
///
/// # Errors
///
/// Rend une erreur si le fichier ne peut pas être écrit.
pub fn ignorer(paths: &Paths, version: &str) -> Result<()> {
    let m = Memoire {
        ignoree: Some(version.to_owned()),
    };
    std::fs::create_dir_all(paths.bdd())?;
    std::fs::write(chemin_memoire(paths), serde_json::to_vec_pretty(&m)?)?;
    Ok(())
}

/// Demande à GitHub s'il y a mieux que `courante`.
///
/// Une requête. Le contrôle échoue sans bruit hors ligne : l'interface ne
/// montre alors rien.
///
/// # Errors
///
/// Rend une erreur si GitHub ne répond pas ou répond mal.
pub async fn verifier(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    courante: &str,
    avec_pre: bool,
) -> Result<Verdict> {
    let releases = ygo_sources::github::releases(client).await?;
    Ok(match choisir(&releases, courante, avec_pre) {
        None => Verdict::AJour,
        Some(d) if memoire(paths).ignoree.as_deref() == Some(d.version.as_str()) => {
            Verdict::Ignoree(d)
        }
        Some(d) => Verdict::Disponible(d),
    })
}

/// L'empreinte lue dans un fichier `.sha256` : les 64 premiers caractères
/// hexadécimaux, quelle que soit la mise en forme (`certutil`, `sha256sum`,
/// `Get-FileHash`).
///
/// ```
/// use ygo_app::maj_appli::lire_empreinte;
/// let h = "a".repeat(64);
/// assert_eq!(lire_empreinte(&format!("{h}  ygo-ui.exe\n")), Some(h.clone()));
/// assert_eq!(lire_empreinte(&h.to_uppercase()), Some(h));
/// assert_eq!(lire_empreinte("pas une empreinte"), None);
/// ```
#[must_use]
pub fn lire_empreinte(texte: &str) -> Option<String> {
    let premier = texte.split_whitespace().next()?;
    let hexa = premier.to_ascii_lowercase();
    (hexa.len() == 64 && hexa.chars().all(|c| c.is_ascii_hexdigit())).then_some(hexa)
}

/// L'empreinte SHA-256 d'octets, en hexadécimal minuscule.
#[must_use]
pub fn empreinte_de(octets: &[u8]) -> String {
    hexa(&Sha256::digest(octets))
}

fn hexa(octets: &[u8]) -> String {
    octets.iter().map(|o| format!("{o:02x}")).collect()
}

/// Où le nouvel exécutable est déposé avant installation.
#[must_use]
pub fn chemin_nouveau(exe: &Path) -> PathBuf {
    suffixe(exe, ".nouveau")
}

/// Où l'exécutable remplacé est mis de côté.
#[must_use]
pub fn chemin_ancien(exe: &Path) -> PathBuf {
    suffixe(exe, ".ancien")
}

fn suffixe(exe: &Path, ajout: &str) -> PathBuf {
    let mut nom = exe.as_os_str().to_owned();
    nom.push(ajout);
    PathBuf::from(nom)
}

/// Télécharge l'exécutable d'une version et vérifie son empreinte, puis sa
/// marque de version.
///
/// `progression(reçus, total)` est appelée à chaque morceau reçu. Rend le
/// chemin du fichier vérifié, à côté de `exe`.
///
/// # Errors
///
/// Rend une erreur si la Release n'est pas installable, si le téléchargement
/// échoue, si l'empreinte ne correspond pas ou si l'exécutable n'est pas de la
/// version annoncée — le fichier est alors effacé.
pub async fn telecharger<F>(
    client: &ygo_sources::ClientHttp,
    dispo: &Disponible,
    exe: &Path,
    mut progression: F,
) -> Result<PathBuf>
where
    F: FnMut(u64, u64),
{
    let (Some((url_exe, taille)), Some(empreinte)) = (&dispo.exe, &dispo.empreinte) else {
        return Err(AppError::MiseAJour(format!(
            "la Release {} ne joint pas {EXE} avec une empreinte vérifiable",
            dispo.tag
        )));
    };

    let attendue = match empreinte {
        Empreinte::Publiee(h) => h.clone(),
        Empreinte::Fichier(url) => {
            let texte = client
                .get_ok(url)
                .await?
                .text()
                .await
                .map_err(|e| ygo_sources::SourceError::reseau(url.as_str(), e))?;
            lire_empreinte(&texte)
                .ok_or_else(|| AppError::MiseAJour(format!("{EMPREINTE} illisible")))?
        }
    };

    let destination = chemin_nouveau(exe);
    let mut reponse = client
        .get_ok_avec_delai(url_exe, ygo_sources::http::DELAI_TELECHARGEMENT)
        .await?;
    let total = reponse.content_length().unwrap_or(*taille);
    let mut fichier = std::fs::File::create(&destination)?;
    let mut hacheur = Sha256::new();
    let mut recus: u64 = 0;
    loop {
        let morceau = match reponse.chunk().await {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => {
                drop(fichier);
                let _ = std::fs::remove_file(&destination);
                return Err(ygo_sources::SourceError::reseau(url_exe.as_str(), e).into());
            }
        };
        hacheur.update(&morceau);
        fichier.write_all(&morceau)?;
        recus += morceau.len() as u64;
        progression(recus, total);
    }
    fichier.flush()?;
    drop(fichier);

    let obtenue = hexa(&hacheur.finalize());
    if obtenue != attendue {
        let _ = std::fs::remove_file(&destination);
        return Err(AppError::MiseAJour(format!(
            "empreinte différente de celle publiée — fichier écarté \
             (attendue {attendue}, reçue {obtenue})"
        )));
    }
    let octets = std::fs::read(&destination)?;
    if let Err(raison) = controler_version(&octets, dispo) {
        let _ = std::fs::remove_file(&destination);
        return Err(AppError::MiseAJour(raison));
    }
    tracing::info!(version = %dispo.version, octets = recus, "nouvel exécutable téléchargé et vérifié");
    Ok(destination)
}

/// Copie les classeurs et les réglages dans `backups/avant_maj_<version>/`.
///
/// La base de référence n'y est pas : elle se reconstruit. Les images non
/// plus : elles se retéléchargent.
///
/// # Errors
///
/// Rend une erreur si la copie échoue.
pub fn sauvegarder(paths: &Paths, version_courante: &str) -> Result<PathBuf> {
    sauvegarder_dans(
        paths,
        &format!("avant_maj_{}", version_courante.replace(['/', '\\'], "_")),
    )
}

/// Copie les classeurs et les réglages dans `backups/<dossier>/`.
///
/// Le geste de [`sauvegarder`], sous un autre nom de dossier — c'est aussi
/// celui du contrôle complet ([`crate::controle`]).
///
/// # Errors
///
/// Rend une erreur si la copie échoue.
pub fn sauvegarder_dans(paths: &Paths, dossier: &str) -> Result<PathBuf> {
    let cible = paths.backups().join(dossier);
    copier_dossier(&paths.classeurs(), &cible.join("classeur_creer"))?;
    for fichier in [paths.app_config(), paths.rarity_config()] {
        if fichier.is_file() {
            if let Some(nom) = fichier.file_name() {
                std::fs::create_dir_all(&cible)?;
                std::fs::copy(&fichier, cible.join(nom))?;
            }
        }
    }
    Ok(cible)
}

fn copier_dossier(source: &Path, cible: &Path) -> Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(cible)?;
    for entree in std::fs::read_dir(source)? {
        let entree = entree?;
        let chemin = entree.path();
        let destination = cible.join(entree.file_name());
        if chemin.is_dir() {
            copier_dossier(&chemin, &destination)?;
        } else {
            std::fs::copy(&chemin, &destination)?;
        }
    }
    Ok(())
}

/// Met le nouvel exécutable à la place de l'actuel.
///
/// L'actuel est **renommé** — Windows le permet pendant qu'il tourne —, puis
/// le nouveau prend son nom. Si ce second geste échoue, l'actuel reprend sa
/// place : on ne laisse jamais l'installation sans exécutable.
///
/// # Errors
///
/// Rend une erreur si l'un des renommages échoue.
pub fn installer(exe: &Path, nouveau: &Path) -> Result<()> {
    let ancien = chemin_ancien(exe);
    if ancien.exists() {
        let _ = std::fs::remove_file(&ancien);
    }
    std::fs::rename(exe, &ancien)?;
    if let Err(e) = std::fs::rename(nouveau, exe) {
        let _ = std::fs::rename(&ancien, exe);
        return Err(AppError::MiseAJour(format!(
            "remplacement de l'exécutable refusé : {e}"
        )));
    }
    Ok(())
}

/// Efface ce qu'une mise à jour précédente a laissé : l'ancien exécutable,
/// un téléchargement interrompu. Silencieux : un fichier encore verrouillé
/// sera effacé au lancement suivant.
pub fn nettoyer(exe: &Path) {
    for reste in [chemin_ancien(exe), chemin_nouveau(exe)] {
        if reste.exists() {
            match std::fs::remove_file(&reste) {
                Ok(()) => tracing::info!(fichier = %reste.display(), "reste de mise à jour effacé"),
                Err(e) => {
                    tracing::debug!(fichier = %reste.display(), erreur = %e, "reste de mise à jour encore verrouillé")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use ygo_sources::github::Fichier;

    fn release(tag: &str, pre: bool, avec_fichiers: bool) -> Release {
        let fichier = |nom: &str| Fichier {
            name: nom.to_owned(),
            browser_download_url: format!("https://exemple/{tag}/{nom}"),
            size: 10,
            digest: None,
        };
        Release {
            tag_name: tag.to_owned(),
            name: Some(format!("{tag} — titre")),
            body: Some("notes".to_owned()),
            prerelease: pre,
            draft: false,
            html_url: format!("https://exemple/{tag}"),
            assets: if avec_fichiers {
                vec![fichier(EXE), fichier(EMPREINTE)]
            } else {
                Vec::new()
            },
        }
    }

    /// Les Releases réelles du dépôt, plus deux à venir.
    fn depot() -> Vec<Release> {
        vec![
            release("v2.1.0-beta.1", true, true),
            release("v2.0.0", false, true),
            release("v2.0.0-alpha.2", true, true),
            release("v2.0.0-alpha.1", false, true),
            release("Ajout Inventaire + Notion", false, false),
            release("V1.0.3", false, false),
        ]
    }

    #[test]
    fn la_plus_recente_stable_est_proposee_sans_les_versions_de_test() {
        let d = choisir(&depot(), "2.0.0-alpha.1", false).unwrap();
        assert_eq!(d.version, "2.0.0");
        assert!(!d.pre_version);
        assert!(d.installable());
    }

    #[test]
    fn avec_les_versions_de_test_la_plus_recente_de_toutes() {
        let d = choisir(&depot(), "2.0.0-alpha.1", true).unwrap();
        assert_eq!(d.version, "2.1.0-beta.1");
        assert!(d.pre_version);
    }

    #[test]
    fn a_jour_ou_plus_recent_que_le_depot_rien_n_est_propose() {
        assert_eq!(choisir(&depot(), "2.1.0-beta.1", true), None);
        assert_eq!(choisir(&depot(), "2.0.0", false), None);
        assert_eq!(choisir(&depot(), "3.0.0", true), None);
    }

    #[test]
    fn une_release_sans_executable_n_est_pas_installable() {
        let d = choisir(&[release("v9.0.0", false, false)], "2.0.0", false).unwrap();
        assert!(!d.installable());
        assert_eq!(d.page, "https://exemple/v9.0.0");
    }

    /// La Release 2.0.1 réelle : l'exe seul, sans `.sha256` — l'empreinte de
    /// GitHub suffit, et passe avant un `.sha256` joint.
    #[test]
    fn l_empreinte_calculee_par_github_rend_la_release_installable() {
        let h = "4f".repeat(32);
        let mut r = release("v2.0.1", false, false);
        r.assets.push(Fichier {
            name: EXE.to_owned(),
            browser_download_url: "https://exemple/ygo-ui.exe".to_owned(),
            size: 10,
            digest: Some(format!("sha256:{h}")),
        });
        let d = choisir(std::slice::from_ref(&r), "2.0.0-alpha.2", false).unwrap();
        assert!(d.installable());
        assert_eq!(d.empreinte, Some(Empreinte::Publiee(h.clone())));

        r.assets.push(Fichier {
            name: EMPREINTE.to_owned(),
            browser_download_url: "https://exemple/sha".to_owned(),
            size: 76,
            digest: None,
        });
        let d = choisir(&[r], "2.0.0-alpha.2", false).unwrap();
        assert_eq!(d.empreinte, Some(Empreinte::Publiee(h)));
    }

    #[test]
    fn sans_empreinte_de_github_le_fichier_sha256_sert() {
        let d = choisir(&[release("v9.0.0", false, true)], "2.0.0", false).unwrap();
        assert_eq!(
            d.empreinte,
            Some(Empreinte::Fichier(format!(
                "https://exemple/v9.0.0/{EMPREINTE}"
            )))
        );
    }

    #[test]
    fn la_marque_de_version_se_retrouve_dans_un_executable() {
        assert_eq!(
            MARQUE,
            format!("YGO-UI-VERSION={};", env!("CARGO_PKG_VERSION"))
        );
        // Le préfixe nu (comme dans le code qui cherche) puis une vraie marque.
        let mut exe = b"\0\0YGO-UI-VERSION=\0junk".to_vec();
        exe.extend_from_slice(b"..YGO-UI-VERSION=2.0.1;..");
        assert_eq!(
            versions_marquees(&exe),
            vec![NumeroVersion::lire("2.0.1").unwrap()]
        );
        assert!(versions_marquees(b"rien").is_empty());
    }

    #[test]
    fn un_executable_d_une_autre_version_est_refuse() {
        let d = choisir(&[release("v2.0.1", false, true)], "2.0.0-alpha.2", false).unwrap();
        assert!(controler_version(b"YGO-UI-VERSION=2.0.1;", &d).is_ok());
        assert!(controler_version(b"YGO-UI-VERSION=v2.0.1;", &d).is_ok());
        let autre = controler_version(b"YGO-UI-VERSION=2.0.0-alpha.1;", &d).unwrap_err();
        assert!(autre.contains("2.0.0-alpha.1"), "{autre}");
        let ancien = controler_version(b"exe sans marque", &d).unwrap_err();
        assert!(ancien.contains("ne porte pas"), "{ancien}");
    }

    #[test]
    fn l_empreinte_se_calcule_comme_get_filehash() {
        // SHA-256 de « abc », vecteur de test du NIST.
        assert_eq!(
            empreinte_de(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Le remplacement de bout en bout, sur de vrais fichiers : l'actuel
    /// devient l'ancien, le nouveau prend sa place, le nettoyage efface
    /// l'ancien.
    #[test]
    fn installer_puis_nettoyer() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(EXE);
        std::fs::write(&exe, b"ancienne version").unwrap();
        let nouveau = chemin_nouveau(&exe);
        std::fs::write(&nouveau, b"nouvelle version").unwrap();

        installer(&exe, &nouveau).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"nouvelle version");
        assert_eq!(
            std::fs::read(chemin_ancien(&exe)).unwrap(),
            b"ancienne version"
        );
        assert!(!nouveau.exists());

        nettoyer(&exe);
        assert!(!chemin_ancien(&exe).exists());
        assert!(exe.exists());
    }

    /// Un nouveau fichier absent : l'actuel reprend sa place, intact.
    #[test]
    fn un_remplacement_rate_remet_l_actuel_en_place() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(EXE);
        std::fs::write(&exe, b"actuel").unwrap();
        assert!(installer(&exe, &tmp.path().join("absent")).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"actuel");
    }

    #[test]
    fn une_version_ignoree_est_retenue() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        assert_eq!(memoire(&paths).ignoree, None);
        ignorer(&paths, "2.0.0").unwrap();
        assert_eq!(memoire(&paths).ignoree.as_deref(), Some("2.0.0"));
    }

    /// Un petit serveur HTTP local qui sert `/exe` et `/sha`, autant de fois
    /// qu'on le lui demande.
    fn serveur(exe: Vec<u8>, sha: String) -> String {
        use std::io::{Read as _, Write as _};
        let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let adresse = format!("http://{}", ecoute.local_addr().unwrap());
        std::thread::spawn(move || {
            for flux in ecoute.incoming() {
                let Ok(mut flux) = flux else { continue };
                let mut tampon = [0u8; 2048];
                let n = flux.read(&mut tampon).unwrap_or(0);
                let requete = String::from_utf8_lossy(&tampon[..n]).to_string();
                let corps: Vec<u8> = if requete.starts_with("GET /exe") {
                    exe.clone()
                } else {
                    sha.clone().into_bytes()
                };
                let entete = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    corps.len()
                );
                let _ = flux.write_all(entete.as_bytes());
                let _ = flux.write_all(&corps);
            }
        });
        adresse
    }

    fn dispo_locale(adresse: &str, taille: u64) -> Disponible {
        Disponible {
            version: "9.9.9".to_owned(),
            tag: "v9.9.9".to_owned(),
            titre: "v9.9.9".to_owned(),
            notes: String::new(),
            page: String::new(),
            pre_version: false,
            exe: Some((format!("{adresse}/exe"), taille)),
            empreinte: Some(Empreinte::Fichier(format!("{adresse}/sha"))),
        }
    }

    /// De bout en bout, sur un vrai serveur : le fichier arrive, son
    /// empreinte correspond, la progression va jusqu'au bout.
    #[tokio::test]
    async fn un_executable_conforme_est_telecharge_et_verifie() {
        let mut contenu = vec![7u8; 300_000];
        contenu.extend_from_slice(b"YGO-UI-VERSION=9.9.9;");
        let taille = contenu.len() as u64;
        let adresse = serveur(
            contenu.clone(),
            format!("{}  ygo-ui.exe", empreinte_de(&contenu)),
        );
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(EXE);
        let client = ygo_sources::ClientHttp::new().unwrap();
        let mut dernier = (0, 0);
        let chemin = telecharger(&client, &dispo_locale(&adresse, taille), &exe, |r, t| {
            dernier = (r, t);
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&chemin).unwrap(), contenu);
        assert_eq!(dernier, (taille, taille));
    }

    /// Empreinte juste mais exécutable d'une autre version : refusé, effacé.
    #[tokio::test]
    async fn un_executable_mal_etiquete_est_refuse_et_efface() {
        let contenu = b"exe YGO-UI-VERSION=2.0.0-alpha.1; fin".to_vec();
        let adresse = serveur(contenu.clone(), empreinte_de(&contenu));
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(EXE);
        let client = ygo_sources::ClientHttp::new().unwrap();
        let mut dispo = dispo_locale(&adresse, contenu.len() as u64);
        dispo.empreinte = Some(Empreinte::Publiee(empreinte_de(&contenu)));
        let r = telecharger(&client, &dispo, &exe, |_, _| {}).await;
        assert!(
            matches!(&r, Err(AppError::MiseAJour(m)) if m.contains("autre version")),
            "{r:?}"
        );
        assert!(!chemin_nouveau(&exe).exists());
    }

    /// Une empreinte qui ne correspond pas : erreur, et rien ne reste.
    #[tokio::test]
    async fn un_executable_altere_est_refuse_et_efface() {
        let adresse = serveur(vec![1, 2, 3], "0".repeat(64));
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(EXE);
        let client = ygo_sources::ClientHttp::new().unwrap();
        let r = telecharger(&client, &dispo_locale(&adresse, 3), &exe, |_, _| {}).await;
        assert!(matches!(r, Err(AppError::MiseAJour(_))));
        assert!(
            !chemin_nouveau(&exe).exists(),
            "le fichier altéré est effacé"
        );
    }

    #[test]
    fn la_sauvegarde_copie_les_classeurs_et_les_reglages() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        std::fs::write(paths.classeur_db("RA02"), b"base").unwrap();
        std::fs::write(paths.app_config(), b"{}").unwrap();
        let cible = sauvegarder(&paths, "2.0.0-alpha.1").unwrap();
        assert_eq!(
            std::fs::read(cible.join("classeur_creer").join("RA02").join("RA02.db")).unwrap(),
            b"base"
        );
        assert!(cible.join("app_config.json").is_file());
    }
}
