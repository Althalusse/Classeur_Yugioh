// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Arborescence des données utilisateur.
//!
//! Portage de `module/centralisation_dossier.py`.
//!
//! **Ces chemins sont des invariants** (§3.3 du cahier des charges), pas des
//! détails d'implémentation : la V2 doit ouvrir une installation V1.0.4 sans
//! conversion, et réutiliser les vignettes déjà téléchargées.
//!
//! ```text
//! <racine>/
//! ├── bdd/
//! │   ├── cardinfo.db                    base de référence (~170 Mo)
//! │   ├── classeur_creer/<CODE>/<CODE>.db un classeur = un dossier + une base
//! │   ├── app_config.json                préférences
//! │   ├── rarity_config.json             priorités de rareté (43 entrées)
//! │   ├── last_update.txt                ⚠ contenu JSON malgré l'extension
//! │   ├── fr_names_cache.json            cache des noms FR
//! │   ├── downloads_actifs.json          file d'attente persistante
//! │   ├── stats_cache.json
//! │   └── anomalies.json
//! ├── img/
//! │   ├── small/<ygoprodeck_image_id>.jpg
//! │   ├── boosters/<CODE_SET>.png
//! │   └── notfound.jpg
//! ├── export/
//! ├── backups/
//! ├── logs/app.log
//! ├── crash_natif.log
//! └── first_run.flag
//! ```
//!
//! La racine est le dossier de l'exécutable, exactement comme en Python
//! (`get_exe_dir()`), afin qu'une V2 posée à côté de la V1.0.4 voie les mêmes
//! données.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// Emplacements de l'arborescence de données, calculés une fois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    racine: PathBuf,
}

impl Paths {
    /// Construit l'arborescence à partir du dossier de l'exécutable courant.
    ///
    /// Équivalent de `centralisation_dossier.get_exe_dir()`.
    pub fn depuis_executable() -> Result<Self> {
        let exe =
            std::env::current_exe().map_err(|e| CoreError::RacineIntrouvable(e.to_string()))?;
        let dossier = exe.parent().ok_or_else(|| {
            CoreError::RacineIntrouvable(format!("{} n'a pas de parent", exe.display()))
        })?;
        Ok(Self::depuis_racine(dossier))
    }

    /// Construit l'arborescence à partir d'une racine explicite.
    ///
    /// Utilisé par les tests et par `ygo-cli`, qui doit pouvoir pointer une
    /// installation V1.0.3 ou V1.0.4 arbitraire pour comparaison.
    pub fn depuis_racine(racine: impl Into<PathBuf>) -> Self {
        Self {
            racine: racine.into(),
        }
    }

    /// Dossier racine (celui de l'exécutable en production).
    pub fn racine(&self) -> &Path {
        &self.racine
    }

    /// `bdd/`
    pub fn bdd(&self) -> PathBuf {
        self.racine.join("bdd")
    }

    /// `bdd/cardinfo.db` — base de référence.
    pub fn cardinfo_db(&self) -> PathBuf {
        self.bdd().join("cardinfo.db")
    }

    /// `bdd/cache_http/` — les réponses d'API gardées et les adresses
    /// introuvables, pour ne pas redemander aux sources ce qu'elles ont déjà
    /// dit (cf. `ygo_sources::cache`). Le supprimer est sans danger.
    pub fn cache_http(&self) -> PathBuf {
        self.bdd().join("cache_http")
    }

    /// `bdd/classeur_creer/`
    pub fn classeurs(&self) -> PathBuf {
        self.bdd().join("classeur_creer")
    }

    /// `bdd/classeur_creer/<CODE>/` — dossier d'un classeur.
    pub fn dossier_classeur(&self, code_set: &str) -> PathBuf {
        self.classeurs().join(code_set)
    }

    /// `bdd/classeur_creer/<CODE>/<CODE>.db` — base d'un classeur.
    ///
    /// Le nom du fichier reprend le code du set, y compris son suffixe OCG
    /// (`LOCH-JP/LOCH-JP.db`).
    pub fn classeur_db(&self, code_set: &str) -> PathBuf {
        self.dossier_classeur(code_set)
            .join(format!("{code_set}.db"))
    }

    /// `bdd/app_config.json` — préférences utilisateur.
    pub fn app_config(&self) -> PathBuf {
        self.bdd().join("app_config.json")
    }

    /// `bdd/rarity_config.json` — priorités de rareté.
    ///
    /// Absent de la liste des invariants du cahier des charges (§3.3/§3.5),
    /// alors que le perdre revient à perdre l'ordre de tri de la collection.
    pub fn rarity_config(&self) -> PathBuf {
        self.bdd().join("rarity_config.json")
    }

    /// `bdd/last_update.txt` — ⚠ contenu **JSON** malgré l'extension `.txt`.
    pub fn last_update(&self) -> PathBuf {
        self.bdd().join("last_update.txt")
    }

    /// `bdd/fr_names_cache.json`
    pub fn fr_names_cache(&self) -> PathBuf {
        self.bdd().join("fr_names_cache.json")
    }

    /// `bdd/downloads_actifs.json` — file d'attente de téléchargement persistante.
    pub fn downloads_actifs(&self) -> PathBuf {
        self.bdd().join("downloads_actifs.json")
    }

    /// `bdd/stats_cache.json`
    pub fn stats_cache(&self) -> PathBuf {
        self.bdd().join("stats_cache.json")
    }

    /// `bdd/anomalies.json`
    pub fn anomalies_json(&self) -> PathBuf {
        self.bdd().join("anomalies.json")
    }

    /// `img/`
    pub fn img(&self) -> PathBuf {
        self.racine.join("img")
    }

    /// `img/small/` — vignettes de cartes.
    pub fn img_small(&self) -> PathBuf {
        self.img().join("small")
    }

    /// `img/small/<ygoprodeck_image_id>.jpg`
    ///
    /// Convention de nommage **invariante** : les vignettes déjà présentes
    /// doivent être réutilisées sans re-téléchargement.
    pub fn vignette(&self, ygoprodeck_image_id: i64) -> PathBuf {
        self.img_small().join(format!("{ygoprodeck_image_id}.jpg"))
    }

    /// `img/boosters/` — covers de boosters.
    pub fn img_boosters(&self) -> PathBuf {
        self.img().join("boosters")
    }

    /// Extensions de cover essayées, dans l'ordre (`find_local_booster`).
    ///
    /// L'URL source peut pointer sur n'importe laquelle : ne chercher que
    /// `.png` ferait manquer des covers réellement présentes.
    pub const EXTENSIONS_COVER: [&'static str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

    /// `img/boosters/<CODE_SET>.<ext>` — le chemin, sans vérifier qu'il existe.
    ///
    /// Pour trouver la cover réellement présente, voir [`Self::chercher_cover`] :
    /// cinq extensions sont possibles, et celle-ci n'en construit qu'une.
    pub fn cover_ext(&self, code_set: &str, extension: &str) -> PathBuf {
        self.img_boosters()
            .join(format!("{}.{extension}", code_set.to_uppercase()))
    }

    /// La cover locale d'un set, si elle existe.
    ///
    /// Portage de `find_local_booster` : les cinq extensions sont essayées dans
    /// l'ordre, et la première présente gagne. Ne télécharge rien — c'est le
    /// `auto_download=False` du Python, celui que l'interface peut appeler sans
    /// bloquer.
    pub fn chercher_cover(&self, code_set: &str) -> Option<PathBuf> {
        Self::EXTENSIONS_COVER
            .iter()
            .map(|ext| self.cover_ext(code_set, ext))
            .find(|chemin| chemin.is_file())
    }

    /// `img/notfound.jpg` — image de remplacement.
    pub fn image_par_defaut(&self) -> PathBuf {
        self.img().join("notfound.jpg")
    }

    /// `export/` — sorties CSV.
    pub fn export(&self) -> PathBuf {
        self.racine.join("export")
    }

    /// `backups/`
    pub fn backups(&self) -> PathBuf {
        self.racine.join("backups")
    }

    /// `logs/` — le Python écrit `logs/app.log`.
    pub fn logs(&self) -> PathBuf {
        self.racine.join("logs")
    }

    /// `logs/app.log`
    pub fn app_log(&self) -> PathBuf {
        self.logs().join("app.log")
    }

    /// `crash_natif.log` — dépôt de panique, à la racine comme en Python.
    pub fn crash_log(&self) -> PathBuf {
        self.racine.join("crash_natif.log")
    }

    /// `first_run.flag`
    pub fn first_run_flag(&self) -> PathBuf {
        self.racine.join("first_run.flag")
    }

    /// Crée les dossiers de l'arborescence s'ils n'existent pas.
    ///
    /// Équivalent de `centralisation_dossier.init_folders()`. Idempotent.
    pub fn creer_dossiers(&self) -> Result<()> {
        for dossier in [
            self.bdd(),
            self.classeurs(),
            self.img(),
            self.img_small(),
            self.img_boosters(),
            self.export(),
            self.backups(),
            self.logs(),
        ] {
            std::fs::create_dir_all(&dossier).map_err(|e| CoreError::io(&dossier, e))?;
        }
        Ok(())
    }

    /// Liste les codes de set des classeurs existants, triés.
    ///
    /// Un classeur est un dossier de `bdd/classeur_creer/` contenant un fichier
    /// `<CODE>.db`. Un dossier sans base est ignoré silencieusement, comme en
    /// Python.
    pub fn classeurs_existants(&self) -> Vec<String> {
        let racine = self.classeurs();
        let Ok(entrees) = std::fs::read_dir(&racine) else {
            return Vec::new();
        };

        let mut codes: Vec<String> = entrees
            .flatten()
            .filter_map(|e| {
                let nom = e.file_name().to_str()?.to_owned();
                if e.path().is_dir() && self.classeur_db(&nom).is_file() {
                    Some(nom)
                } else {
                    None
                }
            })
            .collect();
        codes.sort();
        codes
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn chemins_conformes_a_l_arborescence_v1() {
        let p = Paths::depuis_racine("/app");

        assert_eq!(p.cardinfo_db(), Path::new("/app/bdd/cardinfo.db"));
        assert_eq!(
            p.classeur_db("RA05"),
            Path::new("/app/bdd/classeur_creer/RA05/RA05.db")
        );
        assert_eq!(
            p.classeur_db("LOCH-JP"),
            Path::new("/app/bdd/classeur_creer/LOCH-JP/LOCH-JP.db")
        );
        assert_eq!(p.app_config(), Path::new("/app/bdd/app_config.json"));
        assert_eq!(p.rarity_config(), Path::new("/app/bdd/rarity_config.json"));
        assert_eq!(p.last_update(), Path::new("/app/bdd/last_update.txt"));
        assert_eq!(
            p.vignette(101206062),
            Path::new("/app/img/small/101206062.jpg")
        );
        assert_eq!(
            p.cover_ext("RA05", "png"),
            Path::new("/app/img/boosters/RA05.png")
        );
        assert_eq!(
            p.cover_ext("ra05", "webp"),
            Path::new("/app/img/boosters/RA05.webp"),
            "le code est mis en majuscules, comme find_local_booster"
        );
        assert_eq!(p.app_log(), Path::new("/app/logs/app.log"));
        assert_eq!(p.crash_log(), Path::new("/app/crash_natif.log"));
    }

    #[test]
    fn creer_dossiers_est_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::depuis_racine(tmp.path());

        p.creer_dossiers().unwrap();
        p.creer_dossiers().unwrap(); // deuxième passe : aucun effet, aucune erreur

        assert!(p.classeurs().is_dir());
        assert!(p.img_small().is_dir());
        assert!(p.img_boosters().is_dir());
        assert!(p.backups().is_dir());
        assert!(p.logs().is_dir());
    }

    #[test]
    fn classeurs_existants_ignore_les_dossiers_sans_base() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::depuis_racine(tmp.path());
        p.creer_dossiers().unwrap();

        for code in ["RA05", "LOCH-JP", "VASM"] {
            std::fs::create_dir_all(p.dossier_classeur(code)).unwrap();
            std::fs::write(p.classeur_db(code), b"").unwrap();
        }
        // Dossier orphelin, sans fichier .db : doit être ignoré.
        std::fs::create_dir_all(p.dossier_classeur("VIDE")).unwrap();

        assert_eq!(p.classeurs_existants(), vec!["LOCH-JP", "RA05", "VASM"]);
    }

    #[test]
    fn classeurs_existants_sur_arborescence_absente() {
        let p = Paths::depuis_racine("/chemin/qui/n/existe/pas");
        assert!(p.classeurs_existants().is_empty());
    }
}
