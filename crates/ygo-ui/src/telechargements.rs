// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les téléchargements d'images, en arrière-plan de l'interface.
//!
//! Portage de `file_attente_classeur.FileAttenteClasseur`, moins la création
//! de classeur : ici, une tâche ne fait que compléter les images d'un classeur
//! existant.
//!
//! # Pourquoi un fil séparé, et pas `async` dans la boucle de rendu
//!
//! egui redessine soixante fois par seconde. Une attente réseau dans cette
//! boucle fige la fenêtre — c'est exactement le défaut que le Python évitait
//! avec son worker. Le travail vit donc sur un fil à lui, avec sa propre
//! exécution `tokio`, et ne communique que par messages.
//!
//! L'interface ne partage **aucun état mutable** avec ce fil : elle lit une
//! file d'événements. C'est ce qui rend le service testable sans fenêtre — la
//! machine à états [`Etat`] se nourrit d'événements, d'où qu'ils viennent.
//!
//! # Le réveil de l'interface
//!
//! egui ne redessine que quand quelque chose bouge. Un événement arrivé
//! pendant que la fenêtre dort ne serait vu qu'au prochain mouvement de
//! souris — la progression semblerait figée. Le fil de travail appelle donc
//! [`egui::Context::request_repaint`] après chaque envoi ; `Context` est
//! partageable entre fils, c'est prévu pour cet usage.
//!
//! # Deux fils : les données d'abord, les images ensuite — 2026-10-03
//!
//! Un seul fil faisait tout, dans l'ordre : classeur A créé, **toutes** ses
//! images, puis seulement le classeur B. Avec Yugipedia à une image par
//! seconde, un set de trois cents cartes retenait les suivants cinq minutes ;
//! seize classeurs demandés par un import n'étaient tous là qu'au bout d'une
//! heure, et l'import attendait avec eux.
//!
//! Le travail est désormais coupé en deux fils :
//!
//! - **le fil des données** (`ygo-donnees`) : créations de classeurs —
//!   lignes, couverture, passe artworks —, mise à jour de la base, contrôle de
//!   version. Chaque tâche y dure quelques secondes ; un classeur apparaît à
//!   l'accueil sans attendre les images d'aucun autre ;
//! - **le fil des images** (`ygo-images`) : il reçoit chaque classeur créé
//!   **après** ses données, et le classeur qu'on ouvre **avant** les autres.
//!
//! Les règles des API n'en sont pas affaiblies : quotas, cache et
//! sérialisation Yugipedia sont tenus **pour tout le processus**
//! (`ygo_sources::http`), pas par fil. Deux fils qui frappent Yugipedia se
//! partagent la même seconde ; ils ne la doublent pas.
//!
//! La mise à jour de la base remplace `cardinfo.db`, que le fil des images
//! lit pour préparer chaque passe : il marque une pause le temps qu'elle
//! s'écrive (cf. `Travaux`).
//!
//! # La reprise
//!
//! `bdd/downloads_actifs.json` retient les codes dont les images sont en
//! cours **ou en attente**. Le fil des images le relit à son démarrage et
//! reprend ce qui s'y trouve : une fermeture pendant un téléchargement ne
//! perd rien. Le journal ne mémorise pas ce qui a été téléchargé — la reprise
//! rescanne le disque et ne redemande que ce qui manque encore.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use eframe::egui;
use ygo_core::config::Config;
use ygo_core::paths::Paths;
use ygo_images::{Journal, JournalDisque, Telechargeur};

/// Ce que l'interface demande au fil des images.
enum CommandeImages {
    /// Compléter les images de ce classeur.
    Completer {
        /// Le classeur.
        code: String,
        /// Passer devant ce qui attend : c'est le classeur qu'on regarde.
        ///
        /// Vrai à l'ouverture d'un classeur, faux pour un classeur qui vient
        /// d'être créé — lui rejoint la file à son rang.
        devant: bool,
    },
    /// Télécharger des aperçus **choisis**, hors de toute passe de classeur.
    ///
    /// L'écran des artworks montre des illustrations que le classeur n'a pas
    /// encore : elles ne sont donc dans aucun plan. Elles atterrissent dans
    /// `img/small`, au même nom de fichier que la carte qu'elles deviendront
    /// une fois posées — l'aperçu n'est pas un cache à part, c'est l'image
    /// définitive, téléchargée plus tôt.
    ///
    /// `code` ne sert qu'à nommer la progression : les événements sont ceux
    /// d'une passe ordinaire, et la bande d'avancement les affiche sans rien
    /// savoir de plus.
    Apercus {
        /// Le classeur au nom duquel l'avancement s'affiche.
        code: String,
        /// Les images à chercher.
        cibles: Vec<ygo_images::plan::Cible>,
    },
}

/// Ce que l'interface demande au fil des données.
enum Commande {
    /// Reconstruire `bdd/cardinfo.db`, puis enregistrer sa version.
    ///
    /// C'est « MAJ BDD ». Elle passe par ce fil et pas par un autre : il a
    /// déjà l'exécution `tokio` et le client HTTP, et surtout il est **seul**
    /// — deux mises à jour concurrentes se disputeraient le même fichier.
    ///
    /// Elle ne porte aucun paramètre : depuis l'interface, la source est
    /// toujours le réseau. Le mode hors ligne reste au `ygo-cli`, où il sert
    /// à rejouer une initialisation sur des fixtures figées.
    Init,
    /// Redemander au serveur où en est la version de la base.
    ///
    /// Le fil le fait **de lui-même au démarrage** ; cette commande sert aux
    /// re-contrôles — après une mise à jour, typiquement, où le verdict
    /// affiché est devenu faux.
    VerifierVersion,
    /// Poser l'image Yugipedia de chaque tirage dans tous les classeurs, puis
    /// confier ceux qui ont changé au fil des images.
    ///
    /// C'est le bouton des Options. Sur le fil des données parce qu'il écrit
    /// dans les classeurs — comme une création — et qu'il doit précéder le
    /// téléchargement de ce qu'il pose.
    ImagesYugipedia,
    /// Ajouter à tous les classeurs les numéros que la Set list Yugipedia
    /// connaît et que la base a perdus (cf. `ygo_app::numeros_absents`).
    NumerosAbsents,
    /// Créer ce classeur — lignes, couverture, artworks —, puis passer ses
    /// images au fil des images.
    ///
    /// Le Python enchaînait création **et** images dans la même tâche. On ne
    /// le fait plus : le classeur existe quelques secondes sans ses images —
    /// les cases montrent l'image d'attente —, mais le suivant n'attend plus
    /// qu'elles soient toutes descendues (cf. l'en-tête du module).
    Creer {
        /// Le code du set.
        code: String,
        /// Faut-il compléter les variantes d'illustration via Yugipedia ?
        avec_artworks: bool,
    },
}

/// Le nom sous lequel la mise à jour de la base apparaît dans l'avancement.
///
/// Les tâches sont indexées par code de classeur ; la base n'en est pas un.
/// Un nom qu'aucun set ne peut porter — il contient des espaces — évite toute
/// collision avec un classeur réel, et se lit tel quel dans la bande de
/// progression.
pub const CODE_BASE: &str = "Base de données";

/// Le nom sous lequel la mise à jour des images vers Yugipedia s'annonce.
///
/// Même principe que [`CODE_BASE`] : un nom qu'aucun set ne peut porter.
pub const CODE_IMAGES_YUGIPEDIA: &str = "Images Yugipedia";

/// Le nom sous lequel la recherche des numéros manquants s'annonce.
pub const CODE_NUMEROS_ABSENTS: &str = "Numéros manquants";

/// « La base est en travaux » — partagé entre les deux fils.
///
/// La mise à jour écrit `cardinfo.db` à côté puis la **renomme** à sa place.
/// Sous Windows, ce renommage échoue si un autre fil tient le fichier ouvert ;
/// or le fil des images l'ouvre pour préparer chaque passe (images de repli).
/// Il ne commence donc aucune passe tant que le drapeau est levé.
///
/// Un drapeau et non un verrou : le fil des images n'a rien à protéger
/// pendant qu'il télécharge, il doit seulement ne pas **ouvrir** la base au
/// mauvais moment. La passe déjà commencée continue — elle n'y touche plus.
#[derive(Debug, Clone, Default)]
struct Travaux(Arc<AtomicBool>);

impl Travaux {
    /// Lève le drapeau, et le rabaisse quand la garde tombe — y compris si
    /// la mise à jour panique.
    fn ouvrir(&self) -> GardeTravaux<'_> {
        self.0.store(true, Ordering::SeqCst);
        GardeTravaux(self)
    }

    fn en_cours(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Attend la fin des travaux, en vérifiant toutes les demi-secondes.
    fn attendre_la_fin(&self) {
        while self.en_cours() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

/// Rabaisse le drapeau des travaux en tombant.
struct GardeTravaux<'a>(&'a Travaux);

impl Drop for GardeTravaux<'_> {
    fn drop(&mut self) {
        self.0 .0.store(false, Ordering::SeqCst);
    }
}

/// Où en est la mise à jour de la base.
///
/// L'initialisation émet une dizaine d'étapes typées ([`ygo_app::init::Etape`])
/// dont l'utilisateur n'a que faire : « 12 458 tirages parsés » n'aide pas à
/// patienter. On les replie sur cinq phases, qui sont les cinq attentes
/// réellement distinctes — la seule à durer plusieurs minutes est la
/// première, et c'est la seule à connaître son total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseBase {
    /// L'archive YGOJSON descend du réseau.
    Telechargement,
    /// Le catalogue YGOPRODeck descend à son tour.
    Catalogue,
    /// Cartes et sets se parsent.
    Analyse,
    /// Les variantes d'illustration se résolvent, les cartes s'enrichissent.
    Artworks,
    /// Les tables s'écrivent, en une transaction.
    Ecriture,
}

impl PhaseBase {
    /// Les cinq phases, dans l'ordre où elles se franchissent.
    ///
    /// C'est ce qui permet à un écran de les montrer **toutes** et de dire où
    /// l'on en est — plutôt qu'une seule à la fois, qui n'apprend pas combien
    /// il en reste.
    #[must_use]
    pub fn toutes() -> [Self; 5] {
        [
            Self::Telechargement,
            Self::Catalogue,
            Self::Analyse,
            Self::Artworks,
            Self::Ecriture,
        ]
    }

    /// Le rang de la phase, de 0 à 4.
    #[must_use]
    pub fn rang(self) -> usize {
        match self {
            Self::Telechargement => 0,
            Self::Catalogue => 1,
            Self::Analyse => 2,
            Self::Artworks => 3,
            Self::Ecriture => 4,
        }
    }

    /// Ce que la phase fait, en une ligne.
    #[must_use]
    pub fn detail(self) -> &'static str {
        match self {
            Self::Telechargement => "l'archive YGOJSON, environ 35 Mo",
            Self::Catalogue => "le catalogue YGOPRODeck — facultatif",
            Self::Analyse => "cartes, textes, illustrations et sets",
            Self::Artworks => "variantes d'illustration et statistiques",
            Self::Ecriture => "les tables, en une transaction",
        }
    }

    /// Le libellé montré à l'utilisateur.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Telechargement => "Téléchargement",
            Self::Catalogue => "Catalogue",
            Self::Analyse => "Analyse",
            Self::Artworks => "Artworks",
            Self::Ecriture => "Écriture",
        }
    }
}

/// Ce que le fil de travail rapporte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evenement {
    /// Une passe commence — `total` images à chercher.
    Debut {
        /// Le classeur.
        code: String,
        /// Nombre d'images à télécharger.
        total: usize,
    },
    /// Une image de plus est traitée.
    Progression {
        /// Le classeur.
        code: String,
        /// Images traitées.
        faites: usize,
        /// Total de la passe.
        total: usize,
    },
    /// La passe est finie.
    Fin {
        /// Le classeur.
        code: String,
        /// Images obtenues.
        reussies: usize,
        /// Images qu'aucune source n'a rendues.
        echecs: usize,
    },
    /// Il n'y avait rien à faire — aucun événement de début n'a été émis.
    Complet {
        /// Le classeur.
        code: String,
    },
    /// Une création commence.
    CreationEnCours {
        /// Le classeur.
        code: String,
    },
    /// La complétion des variantes d'illustration commence.
    ArtworksEnCours {
        /// Le classeur.
        code: String,
    },
    /// La couverture du classeur vient d'arriver.
    Couverture {
        /// Le classeur.
        code: String,
    },
    /// Le classeur a été créé.
    Cree {
        /// Le classeur.
        code: String,
        /// Lignes écrites.
        lignes: usize,
    },
    /// Les images de tirage sont posées dans les classeurs.
    ImagesTirage {
        /// Lignes qui ont reçu l'image de leur tirage.
        lignes: usize,
        /// Classeurs touchés — leurs images suivent.
        classeurs: usize,
        /// Fichiers à télécharger, estimés avant de commencer.
        a_telecharger: usize,
    },
    /// Les numéros manquants ont été cherchés dans tous les classeurs.
    NumerosAbsents {
        /// Numéros ajoutés.
        numeros: usize,
        /// Lignes créées.
        lignes: usize,
        /// Classeurs touchés — leurs images suivent.
        classeurs: usize,
        /// Numéros dont la carte reste introuvable dans la base.
        introuvables: usize,
    },
    /// Les données du classeur sont prêtes ; ses images attendent leur tour.
    ///
    /// Le classeur quitte les tâches en cours mais reste dans la file : la
    /// bande le compte encore, et le dit « en attente ».
    ImagesEnAttente {
        /// Le classeur.
        code: String,
    },
    /// La passe n'a pas pu se faire.
    Echec {
        /// Le classeur.
        code: String,
        /// Ce qui s'est passé.
        raison: String,
    },
    /// Le serveur a dit où en est la version de la base.
    Version {
        /// Ce que le contrôle conclut.
        verdict: ygo_app::maj::Verdict,
    },
    /// La mise à jour de la base entre dans une nouvelle phase.
    BaseEnCours {
        /// Où elle en est.
        phase: PhaseBase,
    },
    /// La base est reconstruite.
    BaseTerminee {
        /// Cartes écrites.
        cartes: usize,
        /// Tirages écrits.
        tirages: usize,
        /// La version enregistrée, si le serveur l'a donnée.
        version: Option<String>,
    },
}

impl Evenement {
    /// Le classeur concerné.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Debut { code, .. }
            | Self::Progression { code, .. }
            | Self::Fin { code, .. }
            | Self::Complet { code }
            | Self::CreationEnCours { code }
            | Self::ArtworksEnCours { code }
            | Self::Couverture { code }
            | Self::Cree { code, .. }
            | Self::ImagesEnAttente { code }
            | Self::Echec { code, .. } => code,
            Self::BaseEnCours { .. } | Self::BaseTerminee { .. } | Self::Version { .. } => {
                CODE_BASE
            }
            Self::ImagesTirage { .. } => CODE_IMAGES_YUGIPEDIA,
            Self::NumerosAbsents { .. } => CODE_NUMEROS_ABSENTS,
        }
    }

    /// Cet événement termine-t-il une passe ?
    #[must_use]
    pub fn termine(&self) -> bool {
        matches!(
            self,
            Self::Fin { .. }
                | Self::Complet { .. }
                | Self::Echec { .. }
                | Self::BaseTerminee { .. }
        )
    }
}

/// L'avancement d'une passe en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Avancement {
    /// Images traitées.
    pub faites: usize,
    /// Total de la passe.
    pub total: usize,
}

/// Ce que l'interface sait des téléchargements, à un instant donné.
///
/// Machine à états **pure** : elle ne connaît ni fil, ni réseau, ni disque.
/// C'est elle qu'on éprouve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Etat {
    en_cours: BTreeMap<String, Avancement>,
    /// Dernier message d'issue, à montrer brièvement.
    dernier: Option<String>,
    /// Classeurs dont les images viennent d'arriver, à relire par le cache.
    a_rafraichir: Vec<String>,
    /// Classeurs créés depuis la dernière lecture — l'accueil doit les voir.
    creations: Vec<String>,
    /// À quelle étape en est chaque classeur, pour l'annoncer sans mentir.
    etape: BTreeMap<String, Etape>,
    /// Ce que le serveur a répondu sur la version de la base.
    version: Option<ygo_app::maj::Verdict>,
    /// Codes demandés dont le travail n'est pas terminé — la file d'attente.
    ///
    /// `en_cours` ne porte que ce qui a **commencé** : le fil traite une
    /// commande à la fois, et trois classeurs demandés d'un coup n'y
    /// paraissent qu'un par un. Sans cette file, la bande de progression
    /// annonçait « RA05 » pendant que deux autres attendaient en silence.
    attente: std::collections::BTreeSet<String>,
    /// Classeurs créés dont les images attendent le fil des images.
    ///
    /// Distinct de `attente` : un classeur demandé mais **pas encore créé**
    /// y est aussi, et lui ne doit pas être confié au fil des images.
    attente_images: std::collections::BTreeSet<String>,
    /// Tâches achevées du lot courant.
    faits: usize,
    /// L'utilisateur a écarté le bandeau — pour cette session seulement.
    ///
    /// Écarter n'est pas refuser : le contrôle reste vrai, c'est son
    /// affichage qui se tait. Au prochain démarrage il reparaît, parce qu'une
    /// base périmée le reste.
    maj_ecartee: bool,
}

/// L'étape en cours pour un classeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Etape {
    /// Les lignes du classeur s'écrivent.
    Creation,
    /// Les variantes d'illustration se cherchent sur Yugipedia.
    Artworks,
    /// Les images se téléchargent.
    Images,
    /// Les images de tirage se posent dans les classeurs, avant tout
    /// téléchargement.
    Preparation,
    /// Les classeurs se comparent à leur Set list Yugipedia.
    Verification,
    /// La base de référence se reconstruit.
    ///
    /// Elle porte sa phase : c'est la seule tâche assez longue pour que
    /// « Travail » pendant huit minutes ne dise plus rien.
    Base(PhaseBase),
}

impl Etape {
    /// Le libellé montré à l'utilisateur.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Creation => "Création",
            Self::Artworks => "Artworks",
            Self::Images => "Images",
            Self::Preparation => "Préparation",
            Self::Verification => "Vérification",
            Self::Base(phase) => phase.libelle(),
        }
    }
}

impl Etat {
    /// Applique un événement.
    pub fn appliquer(&mut self, evenement: &Evenement) {
        match evenement {
            Evenement::Debut { code, total } => {
                self.attente_images.remove(code);
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: *total,
                    },
                );
                self.etape.insert(code.clone(), Etape::Images);
            }
            Evenement::Progression {
                code,
                faites,
                total,
            } => {
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: *faites,
                        total: *total,
                    },
                );
                // Une image reçue est une carte de plus à afficher : on le
                // signale sans attendre la fin de la passe, sinon un classeur
                // de cinq cents images resterait troué jusqu'au bout.
                self.a_rafraichir.push(code.clone());
            }
            Evenement::Fin {
                code,
                reussies,
                echecs,
            } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
                self.a_rafraichir.push(code.clone());
                self.dernier = Some(if *echecs == 0 {
                    format!("{code} — {reussies} image(s) récupérée(s)")
                } else {
                    format!("{code} — {reussies} récupérée(s), {echecs} indisponible(s)")
                });
            }
            Evenement::Complet { code } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
            }
            Evenement::Couverture { code } => {
                // L'accueil doit relire : c'est lui qui montre les
                // couvertures, et il les charge une fois pour toutes.
                self.creations.push(code.clone());
            }
            Evenement::ArtworksEnCours { code } => {
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(code.clone(), Etape::Artworks);
            }
            Evenement::CreationEnCours { code } => {
                // Total inconnu tant que les lignes ne sont pas écrites : la
                // barre montre une attente, pas une progression mensongère.
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(code.clone(), Etape::Creation);
            }
            Evenement::Cree { code, lignes } => {
                self.creations.push(code.clone());
                self.dernier = Some(format!("{code} — classeur créé, {lignes} ligne(s)"));
            }
            Evenement::ImagesTirage {
                lignes,
                classeurs,
                a_telecharger,
            } => {
                self.en_cours.remove(CODE_IMAGES_YUGIPEDIA);
                self.etape.remove(CODE_IMAGES_YUGIPEDIA);
                self.dernier = Some(if *lignes == 0 {
                    "Images Yugipedia — tous les classeurs sont déjà à jour".to_owned()
                } else {
                    format!(
                        "Images Yugipedia — {lignes} ligne(s) mise(s) à jour dans \
                         {classeurs} classeur(s) ; ≈ {a_telecharger} image(s) à télécharger"
                    )
                });
            }
            Evenement::NumerosAbsents {
                numeros,
                lignes,
                classeurs,
                introuvables,
            } => {
                self.en_cours.remove(CODE_NUMEROS_ABSENTS);
                self.etape.remove(CODE_NUMEROS_ABSENTS);
                let mut message = if *numeros == 0 {
                    "Numéros manquants — aucun à ajouter".to_owned()
                } else {
                    format!(
                        "Numéros manquants — {numeros} numéro(s) ajouté(s) ({lignes} ligne(s)) \
                         dans {classeurs} classeur(s)"
                    )
                };
                if *introuvables > 0 {
                    message.push_str(&format!(
                        " ; {introuvables} introuvable(s) dans la base, laissé(s) de côté"
                    ));
                }
                self.dernier = Some(message);
            }
            Evenement::ImagesEnAttente { code } => {
                // Plus rien ne tourne pour lui, mais il n'est pas fini : il
                // sort des tâches en cours et reste dans la file d'attente.
                // Compté dans le lot s'il n'y était pas encore — c'est le cas
                // des classeurs que la mise à jour vers Yugipedia confie au
                // fil des images sans être passés par une création.
                self.attendre(code);
                self.attente_images.insert(code.clone());
                self.en_cours.remove(code);
                self.etape.remove(code);
                // La passe artworks a pu ajouter des lignes : l'accueil et le
                // classeur doivent relire.
                self.creations.push(code.clone());
            }
            Evenement::Echec { code, raison } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
                self.dernier = Some(format!("{code} — {raison}"));
            }
            Evenement::Version { verdict } => {
                self.version = Some(verdict.clone());
            }
            Evenement::BaseEnCours { phase } => {
                // Le total repart à zéro à chaque phase : seule la première
                // en connaît un. Les suivantes montrent une barre animée
                // plutôt qu'une progression figée à 100 %.
                self.en_cours.insert(
                    CODE_BASE.to_owned(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(CODE_BASE.to_owned(), Etape::Base(*phase));
            }
            Evenement::BaseTerminee {
                cartes,
                tirages,
                version,
            } => {
                self.en_cours.remove(CODE_BASE);
                self.etape.remove(CODE_BASE);
                // Le verdict d'avant la mise à jour ne veut plus rien dire.
                // Il est effacé ici plutôt que corrigé : le fil en redemande
                // un au serveur, et un verdict deviné n'aurait aucune valeur.
                self.version = None;
                self.maj_ecartee = false;
                // Les classeurs lisent `cardinfo.db` : ils sont tous à relire.
                // L'accueil aussi — les couvertures en dépendent.
                self.creations.push(CODE_BASE.to_owned());
                self.dernier = Some(match version {
                    Some(v) => format!(
                        "Base mise à jour — version {v}, {cartes} cartes, {tirages} tirages"
                    ),
                    None => format!(
                        "Base mise à jour — {cartes} cartes, {tirages} tirages \
                         (version du serveur indisponible)"
                    ),
                });
            }
        }
    }

    /// Y a-t-il quelque chose en cours ?
    #[must_use]
    pub fn actif(&self) -> bool {
        !self.en_cours.is_empty()
    }

    /// Les passes en cours, par code.
    #[must_use]
    pub fn en_cours(&self) -> &BTreeMap<String, Avancement> {
        &self.en_cours
    }

    /// L'avancement cumulé de toutes les passes.
    #[must_use]
    pub fn cumul(&self) -> Avancement {
        self.en_cours.values().fold(
            Avancement {
                faites: 0,
                total: 0,
            },
            |a, b| Avancement {
                faites: a.faites + b.faites,
                total: a.total + b.total,
            },
        )
    }

    /// L'étape en cours d'un classeur.
    #[must_use]
    pub fn etape(&self, code: &str) -> Option<Etape> {
        self.etape.get(code).copied()
    }

    /// Inscrit un code dans la file d'attente.
    ///
    /// Appelé à l'**envoi** de la commande, pas à son premier événement : ce
    /// qui attend son tour doit être compté, sinon la file ne dirait que ce
    /// qui a déjà commencé.
    pub fn attendre(&mut self, code: &str) {
        // Une file entièrement drainée ouvre un lot neuf : sans cela, un
        // classeur demandé une heure plus tard s'annoncerait « 7/7 ».
        if self.attente.is_empty() {
            self.faits = 0;
        }
        self.attente.insert(code.to_owned());
    }

    /// Ce classeur est-il créé, ses images attendant leur tour ?
    #[must_use]
    pub fn images_en_attente(&self, code: &str) -> bool {
        self.attente_images.contains(code)
    }

    /// Retire un code de la file, et compte une tâche de plus.
    fn terminer(&mut self, code: &str) {
        self.attente_images.remove(code);
        if self.attente.remove(code) {
            self.faits += 1;
        }
    }

    /// Le lot en cours : rang de la tâche courante, et total du lot.
    ///
    /// `None` quand il n'y a rien en file, ou qu'il n'y a qu'une seule tâche —
    /// « 1/1 » n'apprend rien que la bande ne dise déjà.
    #[must_use]
    pub fn lot(&self) -> Option<(usize, usize)> {
        let total = self.faits + self.attente.len();
        (total > 1 && !self.attente.is_empty()).then_some((self.faits + 1, total))
    }

    /// Les codes qui attendent leur tour, celui en cours excepté.
    #[must_use]
    pub fn en_attente(&self) -> Vec<&str> {
        self.attente
            .iter()
            .map(String::as_str)
            .filter(|code| !self.en_cours.contains_key(*code))
            .collect()
    }

    /// La base est-elle en cours de reconstruction ?
    #[must_use]
    pub fn base_active(&self) -> bool {
        self.en_cours.contains_key(CODE_BASE)
    }

    /// Le dernier verdict du contrôle de version.
    #[must_use]
    pub fn version(&self) -> Option<&ygo_app::maj::Verdict> {
        self.version.as_ref()
    }

    /// Faut-il montrer le bandeau « mise à jour disponible » ?
    ///
    /// Trois conditions, et les trois comptent : le serveur a conclu qu'il y a
    /// quelque chose à faire, l'utilisateur n'a pas écarté le bandeau, et
    /// aucune mise à jour ne tourne — annoncer « disponible » pendant qu'elle
    /// s'installe serait absurde.
    #[must_use]
    pub fn maj_disponible(&self) -> bool {
        !self.maj_ecartee
            && !self.base_active()
            && matches!(self.version, Some(ygo_app::maj::Verdict::AFaire { .. }))
    }

    /// Écarte le bandeau pour cette session.
    pub fn ecarter_maj(&mut self) {
        self.maj_ecartee = true;
    }

    /// Oublie le verdict courant, en attendant le prochain.
    pub fn oublier_version(&mut self) {
        self.version = None;
    }

    /// Le dernier message d'issue.
    #[must_use]
    pub fn dernier(&self) -> Option<&str> {
        self.dernier.as_deref()
    }

    /// Reprend la liste des classeurs créés, et la vide.
    ///
    /// L'accueil ne se recharge qu'à ce moment-là : un classeur créé doit
    /// apparaître dans la liste sans que l'utilisateur ait à faire quoi que ce
    /// soit.
    pub fn creations(&mut self) -> Vec<String> {
        std::mem::take(&mut self.creations)
    }

    /// Reprend la liste des classeurs à rafraîchir, et la vide.
    ///
    /// L'appelant la consomme : le cache n'oublie ses fichiers absents qu'une
    /// fois par arrivée, pas à chaque image dessinée.
    pub fn a_rafraichir(&mut self) -> Vec<String> {
        std::mem::take(&mut self.a_rafraichir)
    }
}

/// Le service : deux fils de travail, et la file d'événements qui en revient.
pub struct Service {
    /// Vers le fil des données.
    commandes: Sender<Commande>,
    /// Vers le fil des images.
    images: Sender<CommandeImages>,
    evenements: Receiver<Evenement>,
    etat: Etat,
    /// Codes déjà demandés — on ne relance pas une passe à chaque
    /// aller-retour sur le même classeur.
    demandes: std::collections::HashSet<String>,
    /// Une mise à jour de la base est partie et n'est pas revenue.
    ///
    /// Le drapeau est levé à l'**envoi**, pas au premier événement : entre
    /// les deux il y a le temps d'ouvrir une connexion HTTP, et deux clics
    /// dans cet intervalle mettraient deux reconstructions à la file.
    base_demandee: bool,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").field("etat", &self.etat).finish()
    }
}

impl Service {
    /// Démarre le fil de travail.
    ///
    /// `ctx` sert à réveiller l'interface quand un événement arrive.
    #[must_use]
    pub fn demarrer(racine: PathBuf, ctx: egui::Context) -> Self {
        let (envoi_commandes, reception_commandes) = std::sync::mpsc::channel();
        let (envoi_images, reception_images) = std::sync::mpsc::channel();
        let (envoi_evenements, reception_evenements) = std::sync::mpsc::channel();
        let travaux = Travaux::default();

        // Le contrôle de version part sur SON fil, pas sur celui des
        // téléchargements.
        //
        // Le fil de travail traite une commande à la fois, et commence par
        // reprendre les téléchargements interrompus. Un contrôle placé devant
        // aurait retardé cette reprise du temps de la requête — soit, hors
        // ligne, trois tentatives et deux retraits exponentiels, une bonne
        // demi-minute à chaque lancement. Placé derrière, il aurait attendu
        // que des centaines d'images finissent de descendre.
        //
        // Ce contrôle ne dépend de rien et ne bloque rien : une requête, un
        // événement, et le fil meurt. Il n'a aucune raison de partager la
        // file.
        let racine_version = racine.clone();
        let evenements_version = envoi_evenements.clone();
        let ctx_version = ctx.clone();
        std::thread::Builder::new()
            .name("ygo-version".to_owned())
            .spawn(move || controler_version(&racine_version, &evenements_version, &ctx_version))
            .map_or_else(
                |e| tracing::error!(erreur = %e, "fil de contrôle de version non démarré"),
                |_| (),
            );

        let donnees = Fil {
            racine: racine.clone(),
            evenements: envoi_evenements.clone(),
            ctx: ctx.clone(),
            travaux: travaux.clone(),
        };
        let vers_images = envoi_images.clone();
        std::thread::Builder::new()
            .name("ygo-donnees".to_owned())
            .spawn(move || travailler_donnees(&donnees, &reception_commandes, &vers_images))
            .map_or_else(
                |e| tracing::error!(erreur = %e, "fil des données non démarré"),
                |_| (),
            );

        let images = Fil {
            racine,
            evenements: envoi_evenements,
            ctx,
            travaux,
        };
        std::thread::Builder::new()
            .name("ygo-images".to_owned())
            .spawn(move || travailler_images(&images, &reception_images))
            .map_or_else(
                |e| tracing::error!(erreur = %e, "fil des images non démarré"),
                |_| (),
            );

        Self {
            commandes: envoi_commandes,
            images: envoi_images,
            evenements: reception_evenements,
            etat: Etat::default(),
            demandes: std::collections::HashSet::new(),
            base_demandee: false,
        }
    }

    /// Demande la reconstruction de `bdd/cardinfo.db` — « MAJ BDD ».
    ///
    /// Sans effet si une mise à jour est déjà en route.
    pub fn mettre_a_jour_base(&mut self) {
        if self.base_demandee {
            return;
        }
        if self.commandes.send(Commande::Init).is_err() {
            tracing::warn!("fil de téléchargement absent — mise à jour ignorée");
            return;
        }
        self.base_demandee = true;
    }

    /// Une mise à jour de la base est-elle en cours ?
    #[must_use]
    pub fn base_en_cours(&self) -> bool {
        self.base_demandee
    }

    /// Redemande au serveur où en est la version de la base.
    ///
    /// Le fil le fait déjà à son démarrage : ceci est pour les re-contrôles.
    pub fn verifier_version(&mut self) {
        if self.commandes.send(Commande::VerifierVersion).is_err() {
            tracing::warn!("fil de téléchargement absent — contrôle de version ignoré");
            return;
        }
        // Le verdict d'avant est oublié TOUT DE SUITE : l'écran doit dire
        // « contrôle en cours » jusqu'à la réponse, et non réafficher
        // l'ancienne conclusion comme si de rien n'était.
        self.etat.oublier_version();
    }

    /// Écarte le bandeau de mise à jour pour cette session.
    pub fn ecarter_maj(&mut self) {
        self.etat.ecarter_maj();
    }

    /// Demande de poser l'image Yugipedia de chaque tirage dans tous les
    /// classeurs ; leurs images suivent sur le fil des images.
    pub fn images_vers_yugipedia(&mut self) {
        // Total inconnu : barre animée, étape nommée.
        self.etat.en_cours.insert(
            CODE_IMAGES_YUGIPEDIA.to_owned(),
            Avancement {
                faites: 0,
                total: 0,
            },
        );
        self.etat
            .etape
            .insert(CODE_IMAGES_YUGIPEDIA.to_owned(), Etape::Preparation);
        if self.commandes.send(Commande::ImagesYugipedia).is_err() {
            tracing::warn!("fil des données absent — mise à jour des images ignorée");
        }
    }

    /// Demande de chercher, dans tous les classeurs, les numéros que la Set
    /// list Yugipedia connaît et que la base a perdus.
    pub fn numeros_absents(&mut self) {
        self.etat.en_cours.insert(
            CODE_NUMEROS_ABSENTS.to_owned(),
            Avancement {
                faites: 0,
                total: 0,
            },
        );
        self.etat
            .etape
            .insert(CODE_NUMEROS_ABSENTS.to_owned(), Etape::Verification);
        if self.commandes.send(Commande::NumerosAbsents).is_err() {
            tracing::warn!("fil des données absent — recherche des numéros ignorée");
        }
    }

    /// Demande de créer un classeur ; ses images suivront, sur l'autre fil.
    pub fn creer(&mut self, code: &str, avec_artworks: bool) {
        let code = code.trim().to_uppercase();
        // La création marque aussi le code comme demandé : la passe d'images
        // qui la suit ne doit pas être relancée à l'ouverture du classeur —
        // seulement remontée en tête (cf. `completer`).
        self.demandes.insert(code.clone());
        self.etat.attendre(&code);
        if self
            .commandes
            .send(Commande::Creer {
                code,
                avec_artworks,
            })
            .is_err()
        {
            tracing::warn!("fil de téléchargement absent — création ignorée");
        }
    }

    /// Demande le téléchargement d'aperçus.
    ///
    /// Contrairement à [`completer`](Self::completer), la demande **n'est pas
    /// dédoublonnée par classeur** : elle porte sur une liste précise, et
    /// deux appels successifs visent des images différentes.
    pub fn apercus(&mut self, code: &str, cibles: Vec<ygo_images::plan::Cible>) {
        if cibles.is_empty() {
            return;
        }
        let code = code.trim().to_uppercase();
        if self
            .images
            .send(CommandeImages::Apercus { code, cibles })
            .is_err()
        {
            tracing::warn!("fil des images absent — aperçus ignorés");
        }
    }

    /// Demande de compléter les images d'un classeur qu'on ouvre.
    ///
    /// Sans effet si ce classeur a déjà été demandé pendant cette session :
    /// l'ouverture d'un classeur déclenche l'appel, et l'utilisateur y revient
    /// souvent. La demande passe **devant** les classeurs en attente : c'est
    /// celui qu'on regarde.
    ///
    /// Un classeur **déjà** demandé — créé un peu plus tôt, typiquement — dont
    /// les images attendent encore leur tour est remonté en tête lui aussi.
    /// Pas un classeur encore en création : ses données ne sont pas écrites,
    /// le fil des images n'aurait rien à lire.
    pub fn completer(&mut self, code: &str) {
        let code = code.trim().to_uppercase();
        if self.demandes.insert(code.clone()) {
            self.etat.attendre(&code);
        } else if !self.etat.images_en_attente(&code) {
            return;
        }
        if self
            .images
            .send(CommandeImages::Completer { code, devant: true })
            .is_err()
        {
            tracing::warn!("fil des images absent — demande ignorée");
        }
    }

    /// Vide la file d'événements dans l'état, et rend ce qui est arrivé.
    pub fn recevoir(&mut self) -> Vec<Evenement> {
        let mut recus = Vec::new();
        // `try_recv` et non `iter` : la boucle doit rendre la main dès que la
        // file est vide, pas attendre le prochain événement. Une image bloquée
        // sur le réseau figerait sinon tout le rendu.
        while let Ok(e) = self.evenements.try_recv() {
            if e.code() == CODE_BASE && e.termine() {
                self.base_demandee = false;
            }
            self.etat.appliquer(&e);
            recus.push(e);
        }
        recus
    }

    /// L'état courant.
    #[must_use]
    pub fn etat(&self) -> &Etat {
        &self.etat
    }

    /// L'état courant, modifiable — pour reprendre la liste à rafraîchir.
    pub fn etat_mut(&mut self) -> &mut Etat {
        &mut self.etat
    }
}

/// La phase à afficher pour une étape d'initialisation.
///
/// Repli volontaire : dix étapes typées deviennent cinq attentes. `None` pour
/// celles qui ne changent pas de phase — les rapporter ferait clignoter le
/// libellé sans rien apprendre.
fn phase_de(etape: &ygo_app::init::Etape) -> Option<PhaseBase> {
    use ygo_app::init::Etape as E;
    match etape {
        E::TelechargementArchive { .. } => Some(PhaseBase::Telechargement),
        E::ArchiveRecue { .. } => Some(PhaseBase::Catalogue),
        E::CatalogueRecu { .. } | E::CatalogueIndisponible => Some(PhaseBase::Analyse),
        E::ArtworksResolus { .. } | E::CartesEnrichies { .. } => Some(PhaseBase::Artworks),
        E::EcritureBase => Some(PhaseBase::Ecriture),
        // Déjà dans la phase « Analyse » : la traverser ne la change pas.
        E::CartesParsees { .. } | E::SetsParses { .. } | E::Terminee(_) => None,
    }
}

/// La passe artworks d'un classeur qui vient de naître.
///
/// Détachée de `creation::creer` pour que la création rende la main dès que
/// les lignes sont écrites — cf. le commentaire de `Commande::Creer`.
/// La règle des fausses raretés est **dans** `ygo-app` : cette passe ajoute
/// des tirages qui peuvent démasquer un fantôme, et les deux gestes ne doivent
/// plus pouvoir se séparer — cf. `creation::completer_artworks`.
async fn passe_artworks(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
    raretes: &ygo_core::rarity::Priorites,
) -> Result<(), Box<dyn std::error::Error>> {
    ygo_app::creation::completer_artworks(paths, client, code, raretes).await?;
    Ok(())
}

/// Demande au serveur où en est la version de la base, une fois, puis rend la
/// main.
///
/// Un fil à lui, une requête, un événement. Hors ligne, il échoue en quelques
/// secondes et émet un [`Verdict::Indecidable`](ygo_app::maj::Verdict) — ce
/// qui n'empêche rien : le bouton de mise à jour reste actionnable, seul le
/// bandeau se tait.
fn controler_version(
    racine: &std::path::Path,
    evenements: &Sender<Evenement>,
    ctx: &egui::Context,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(erreur = %e, "contrôle de version : exécution tokio non démarrée");
            return;
        }
    };
    let client = match ygo_sources::ClientHttp::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(erreur = %e, "contrôle de version : client HTTP non construit");
            return;
        }
    };
    let paths = Paths::depuis_racine(racine);
    let verdict = runtime.block_on(ygo_app::maj::verifier(&paths, &client));
    let _ = evenements.send(Evenement::Version { verdict });
    ctx.request_repaint();
}

/// Exécute « MAJ BDD », en convertissant sa progression en événements.
///
/// # Pourquoi une pompe, et pas un simple `block_on`
///
/// [`ygo_app::maj::mettre_a_jour`] rend sa progression par un canal `tokio`,
/// qu'il faut vider **pendant** qu'elle travaille — sinon la barre resterait
/// figée huit minutes puis afficherait tout d'un coup. Les deux futures sont
/// donc jointes, et l'émetteur est **déplacé dans** celle du travail : sans
/// cela il resterait vivant après la fin, la pompe attendrait un message qui
/// ne viendrait jamais, et le fil ne rendrait plus la main.
fn executer_maj(
    runtime: &tokio::runtime::Runtime,
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    signaler: &impl Fn(Evenement),
) {
    signaler(Evenement::BaseEnCours {
        phase: PhaseBase::Telechargement,
    });

    let resultat = runtime.block_on(async {
        let (envoi, mut reception) = tokio::sync::mpsc::unbounded_channel();
        let options = ygo_app::init::Options::default();

        let travail = async move {
            let r = ygo_app::maj::mettre_a_jour(paths, client, &options, Some(&envoi)).await;
            drop(envoi); // ferme la pompe : sans ce `drop`, elle attendrait toujours
            r
        };

        let pompe = async {
            let mut phase = PhaseBase::Telechargement;
            while let Some(etape) = reception.recv().await {
                if let Some(nouvelle) = phase_de(&etape) {
                    if nouvelle != phase {
                        phase = nouvelle;
                        signaler(Evenement::BaseEnCours { phase });
                    }
                }
                // Seul le téléchargement connaît son total : c'est la seule
                // étape qui puisse remplir une barre sans l'inventer.
                if let ygo_app::init::Etape::TelechargementArchive { recus, total } = etape {
                    if let Some(total) = total.filter(|t| *t > 0) {
                        signaler(Evenement::Progression {
                            code: CODE_BASE.to_owned(),
                            faites: usize::try_from(recus).unwrap_or(usize::MAX),
                            total: usize::try_from(total).unwrap_or(usize::MAX),
                        });
                    }
                }
            }
        };

        let (resultat, ()) = tokio::join!(travail, pompe);
        resultat
    });

    match resultat {
        Ok(bilan) => {
            let version = bilan.version.clone();
            signaler(Evenement::BaseTerminee {
                cartes: bilan.comptages.cartes,
                tirages: bilan.comptages.tirages,
                version: bilan.version,
            });
            // Le nouveau verdict, SANS redemander au serveur.
            //
            // `maj` vient d'interroger `checkDBVer.php` pour écrire
            // `last_update.txt` : sa réponse est dans le bilan. Un second
            // appel donnait deux requêtes à la même seconde — visible dans le
            // journal du 2026-09-05 — pour reconstruire une conclusion que
            // l'on tenait déjà.
            //
            // Ce n'est pas déduire « à jour » d'un succès : c'est la version
            // que le serveur a donnée et que l'on a écrite. Quand elle
            // manque, le serveur n'a pas répondu — et là il faut bien
            // redemander.
            let verdict = match version {
                Some(version) => ygo_app::maj::Verdict::AJour { version },
                None => runtime.block_on(ygo_app::maj::verifier(paths, client)),
            };
            signaler(Evenement::Version { verdict });
        }
        Err(e) => signaler(Evenement::Echec {
            code: CODE_BASE.to_owned(),
            raison: e.to_string(),
        }),
    }
}

/// Ce que partagent les deux fils de travail.
struct Fil {
    /// La racine de l'installation.
    racine: PathBuf,
    /// La file d'événements vers l'interface — commune aux deux fils.
    evenements: Sender<Evenement>,
    /// De quoi réveiller l'interface.
    ctx: egui::Context,
    /// La base est-elle en travaux ?
    travaux: Travaux,
}

impl Fil {
    /// Envoie un événement, et réveille l'interface pour qu'elle le voie.
    fn signaler(&self, evenement: Evenement) {
        let _ = self.evenements.send(evenement);
        self.ctx.request_repaint();
    }
}

/// L'exécution `tokio` et le client HTTP d'un fil.
///
/// Chaque fil a les siens : une exécution `current_thread` ne se partage pas
/// entre fils. Le client non plus n'a pas à l'être — les quotas et le cache
/// qu'il applique sont ceux du processus, pas les siens.
fn outillage(fil: &str) -> Option<(tokio::runtime::Runtime, ygo_sources::ClientHttp)> {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(fil, erreur = %e, "exécution tokio non démarrée");
            return None;
        }
    };
    match ygo_sources::ClientHttp::new() {
        Ok(client) => Some((runtime, client)),
        Err(e) => {
            tracing::error!(fil, erreur = %e, "client HTTP non construit");
            None
        }
    }
}

/// La boucle du fil des données : créations, mise à jour de la base,
/// contrôles de version.
///
/// Chaque classeur créé est confié **ensuite** au fil des images ; ce fil-ci
/// passe aussitôt à la commande suivante.
fn travailler_donnees(fil: &Fil, commandes: &Receiver<Commande>, images: &Sender<CommandeImages>) {
    let Some((runtime, client)) = outillage("données") else {
        return;
    };
    let paths = Paths::depuis_racine(&fil.racine);
    let raretes = ygo_core::rarity::Priorites::charger(paths.rarity_config());
    let signaler = |e: Evenement| fil.signaler(e);

    while let Ok(commande) = commandes.recv() {
        match commande {
            Commande::Init => {
                // Le fil des images n'ouvre pas `cardinfo.db` tant que la
                // garde vit — cf. `Travaux`.
                let _garde = fil.travaux.ouvrir();
                executer_maj(&runtime, &paths, &client, &signaler);
            }
            Commande::VerifierVersion => {
                signaler(Evenement::Version {
                    verdict: runtime.block_on(ygo_app::maj::verifier(&paths, &client)),
                });
            }
            Commande::ImagesYugipedia => {
                images_vers_yugipedia(&paths, &raretes, images, &signaler);
            }
            Commande::NumerosAbsents => {
                numeros_absents_partout(&runtime, &paths, &client, &raretes, images, &signaler);
            }
            Commande::Creer {
                code,
                avec_artworks,
            } => {
                let cree = creer_classeur(
                    &runtime,
                    &paths,
                    &client,
                    &raretes,
                    &code,
                    avec_artworks,
                    &signaler,
                );
                if cree {
                    // Les données sont prêtes : le classeur passe la main au
                    // fil des images, et attend son tour sans rien retenir.
                    signaler(Evenement::ImagesEnAttente { code: code.clone() });
                    if images
                        .send(CommandeImages::Completer {
                            code,
                            devant: false,
                        })
                        .is_err()
                    {
                        tracing::warn!("fil des images absent — images non demandées");
                    }
                }
            }
        }
    }
}

/// Pose l'image de chaque tirage partout, puis confie au fil des images les
/// classeurs qui ont changé.
///
/// Aucun réseau ici : tout se lit dans `cardinfo.db`. Le réseau, c'est le fil
/// des images qui s'en charge, au rythme que Yugipedia autorise.
fn images_vers_yugipedia(
    paths: &Paths,
    raretes: &ygo_core::rarity::Priorites,
    images: &Sender<CommandeImages>,
    signaler: &impl Fn(Evenement),
) {
    match ygo_app::images_tirage::installation(paths, raretes, true) {
        Ok(bilan) => {
            signaler(Evenement::ImagesTirage {
                lignes: bilan.a_poser,
                classeurs: bilan.touches.len(),
                a_telecharger: bilan.a_telecharger,
            });
            for code in bilan.touches {
                signaler(Evenement::ImagesEnAttente { code: code.clone() });
                if images
                    .send(CommandeImages::Completer {
                        code,
                        devant: false,
                    })
                    .is_err()
                {
                    tracing::warn!("fil des images absent — images non demandées");
                    break;
                }
            }
        }
        Err(e) => signaler(Evenement::Echec {
            code: CODE_IMAGES_YUGIPEDIA.to_owned(),
            raison: e.to_string(),
        }),
    }
}

/// Ajoute à chaque classeur les numéros que la base a perdus, puis confie au
/// fil des images ceux qui ont changé.
///
/// Un classeur qui reçoit des numéros repasse par la passe artworks : ses
/// nouvelles lignes y reçoivent l'image de leur tirage, Overframe compris.
/// Réseau : la Set list de chaque classeur — en cache trente jours —, et une
/// requête pour cinquante noms à résoudre, au rythme de Yugipedia.
fn numeros_absents_partout(
    runtime: &tokio::runtime::Runtime,
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    raretes: &ygo_core::rarity::Priorites,
    images: &Sender<CommandeImages>,
    signaler: &impl Fn(Evenement),
) {
    let (mut numeros, mut lignes, mut introuvables) = (0usize, 0usize, 0usize);
    let mut touches: Vec<String> = Vec::new();
    for code in paths.classeurs_existants() {
        let issue = runtime.block_on(ygo_app::numeros_absents::completer(
            paths, client, &code, raretes, true,
        ));
        let bilan = match issue {
            Ok(ygo_app::numeros_absents::Issue::Fait(b)) => b,
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "numéros manquants non vérifiés");
                continue;
            }
        };
        introuvables += bilan.introuvables.len();
        // Des images posées sans numéro ajouté : un numéro d'un passage
        // précédent, dont les tirages en variante attendent la passe.
        if bilan.ajouts.is_empty() && bilan.images == 0 {
            continue;
        }
        numeros += bilan.ajouts.len();
        lignes += bilan.lignes();
        if let Err(e) = runtime.block_on(ygo_app::creation::completer_artworks(
            paths, client, &code, raretes,
        )) {
            tracing::warn!(classeur = %code, erreur = %e, "passe artworks après ajout");
        }
        touches.push(code);
    }
    signaler(Evenement::NumerosAbsents {
        numeros,
        lignes,
        classeurs: touches.len(),
        introuvables,
    });
    for code in touches {
        signaler(Evenement::ImagesEnAttente { code: code.clone() });
        if images
            .send(CommandeImages::Completer {
                code,
                devant: false,
            })
            .is_err()
        {
            tracing::warn!("fil des images absent — images non demandées");
            break;
        }
    }
}

/// Crée un classeur — lignes, couverture, artworks —, sans ses images.
///
/// Rend `true` si le classeur a été créé et attend désormais ses images.
fn creer_classeur(
    runtime: &tokio::runtime::Runtime,
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    raretes: &ygo_core::rarity::Priorites,
    code: &str,
    avec_artworks: bool,
    signaler: &impl Fn(Evenement),
) -> bool {
    signaler(Evenement::CreationEnCours {
        code: code.to_owned(),
    });
    // `avec_artworks: false` À DESSEIN, même quand l'utilisateur les demande.
    //
    // `creation::creer` enchaîne la passe artworks **avant** de rendre la
    // main : l'événement `Cree` n'arrivait donc qu'après plusieurs minutes de
    // réseau, et le classeur n'apparaissait à l'accueil qu'à ce moment-là. En
    // coupant après l'écriture des lignes, la tuile apparaît en quelques
    // secondes, et les artworks sont une étape visible de plus.
    match runtime.block_on(ygo_app::creation::creer(
        paths,
        client,
        code,
        false,
        &ygo_app::creation::Greffons::default(),
    )) {
        Ok(ygo_app::creation::Issue::Cree { lignes, .. }) => {
            // Annoncé DÈS ICI : les lignes sont écrites, le classeur existe,
            // l'accueil peut le montrer. Ce qui suit le complète mais ne
            // conditionne plus son apparition.
            signaler(Evenement::Cree {
                code: code.to_owned(),
                lignes,
            });
            // La couverture, avant les artworks : c'est ce que l'accueil
            // montre, et elle coûte une seule requête.
            match runtime.block_on(ygo_app::couverture::telecharger(paths, client, code)) {
                Ok(Some(_)) => signaler(Evenement::Couverture {
                    code: code.to_owned(),
                }),
                Ok(None) => {}
                Err(e) => tracing::warn!(classeur = %code, erreur = %e, "couverture"),
            }
            if avec_artworks {
                signaler(Evenement::ArtworksEnCours {
                    code: code.to_owned(),
                });
                if let Err(e) = runtime.block_on(passe_artworks(paths, client, code, raretes)) {
                    tracing::warn!(classeur = %code, erreur = %e, "passe artworks");
                }
            }
            true
        }
        Ok(ygo_app::creation::Issue::DejaExistant) => {
            signaler(Evenement::Echec {
                code: code.to_owned(),
                raison: "le classeur existe déjà".to_owned(),
            });
            false
        }
        Err(e) => {
            signaler(Evenement::Echec {
                code: code.to_owned(),
                raison: e.to_string(),
            });
            false
        }
    }
}

/// Une tâche du fil des images.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TacheImages {
    /// Compléter les images d'un classeur.
    Classeur(String),
    /// Des aperçus choisis.
    Apercus {
        /// Au nom de quel classeur l'avancement s'affiche.
        code: String,
        /// Les images à chercher.
        cibles: Vec<ygo_images::plan::Cible>,
    },
}

/// Range une commande dans la file du fil des images.
///
/// - les aperçus passent devant : l'utilisateur les attend à l'écran ;
/// - un classeur qu'on ouvre passe devant, un classeur créé prend son rang ;
/// - un classeur déjà en file n'y est pas mis deux fois — il est **déplacé**
///   s'il doit passer devant.
///
/// Fonction pure sur la file : c'est elle qu'on éprouve.
fn ranger(file: &mut VecDeque<TacheImages>, commande: CommandeImages) {
    match commande {
        CommandeImages::Apercus { code, cibles } => {
            file.push_front(TacheImages::Apercus { code, cibles });
        }
        CommandeImages::Completer { code, devant } => {
            let deja = file
                .iter()
                .position(|t| matches!(t, TacheImages::Classeur(c) if *c == code));
            match (deja, devant) {
                (Some(_), false) => {}
                (Some(rang), true) => {
                    if let Some(tache) = file.remove(rang) {
                        file.push_front(tache);
                    }
                }
                (None, true) => file.push_front(TacheImages::Classeur(code)),
                (None, false) => file.push_back(TacheImages::Classeur(code)),
            }
        }
    }
}

/// La boucle du fil des images.
///
/// Entre deux tâches, il relit tout ce qui est arrivé : un classeur ouvert
/// pendant qu'un autre se télécharge passe ainsi en tête **dès la passe
/// suivante** — jamais au milieu d'une, qui irait sinon se reprendre de
/// zéro.
fn travailler_images(fil: &Fil, commandes: &Receiver<CommandeImages>) {
    let Some((runtime, client)) = outillage("images") else {
        return;
    };
    let paths = Paths::depuis_racine(&fil.racine);
    let journal = JournalDisque::nouveau(paths.downloads_actifs());

    // La reprise, avant toute commande : ce qui restait en cours — ou en
    // attente — à la fermeture précédente repart en premier.
    let mut file: VecDeque<TacheImages> = journal
        .charger()
        .into_iter()
        .map(TacheImages::Classeur)
        .collect();
    if !file.is_empty() {
        tracing::info!(classeurs = ?file, "reprise des téléchargements interrompus");
    }

    loop {
        while let Ok(commande) = commandes.try_recv() {
            noter(&journal, &commande);
            ranger(&mut file, commande);
        }
        let Some(tache) = file.pop_front() else {
            match commandes.recv() {
                Ok(commande) => {
                    noter(&journal, &commande);
                    ranger(&mut file, commande);
                    continue;
                }
                Err(_) => break, // l'interface est partie
            }
        };
        // Jamais pendant que la base se réécrit : la passe l'ouvre.
        fil.travaux.attendre_la_fin();
        match tache {
            TacheImages::Apercus { code, cibles } => {
                apercus(fil, &runtime, &client, &paths, &code, &cibles);
            }
            TacheImages::Classeur(code) => {
                // La source se relit à chaque passe : on a pu la changer dans
                // les Options depuis le démarrage du fil.
                let source = Config::charger(paths.app_config()).source_image();
                images_du_classeur(fil, &runtime, &client, &paths, source, &journal, code);
            }
        }
    }
}

/// Inscrit un classeur au journal de reprise dès qu'il entre en file.
///
/// Un classeur qui attend son tour doit survivre à une fermeture : sinon,
/// créé puis l'application fermée avant ses images, il n'en aurait aucune
/// jusqu'à ce qu'on l'ouvre.
fn noter(journal: &JournalDisque, commande: &CommandeImages) {
    if let CommandeImages::Completer { code, .. } = commande {
        journal.ajouter(code);
    }
}

/// Des aperçus choisis, hors de toute passe de classeur.
///
/// Ils ne passent pas par le journal de reprise : ils ne sont pas un état à
/// rattraper, seulement un confort d'affichage. Interrompus, ils se
/// redemandent à la réouverture de l'écran.
fn apercus(
    fil: &Fil,
    runtime: &tokio::runtime::Runtime,
    client: &ygo_sources::ClientHttp,
    paths: &Paths,
    code: &str,
    cibles: &[ygo_images::plan::Cible],
) {
    let telechargeur = Telechargeur::nouveau(client, &paths.image_par_defaut());
    fil.signaler(Evenement::Debut {
        code: code.to_owned(),
        total: cibles.len(),
    });
    let bilan = runtime.block_on(telechargeur.toutes(cibles, |faites, total| {
        fil.signaler(Evenement::Progression {
            code: code.to_owned(),
            faites,
            total,
        });
    }));
    ygo_app::replis::consigner(paths, &telechargeur.replis());
    fil.signaler(Evenement::Fin {
        code: code.to_owned(),
        reussies: bilan.reussies,
        echecs: bilan.echecs,
    });
}

/// Les images d'un classeur : couverture, puis ce qui manque sur le disque.
fn images_du_classeur(
    fil: &Fil,
    runtime: &tokio::runtime::Runtime,
    client: &ygo_sources::ClientHttp,
    paths: &Paths,
    source: ygo_core::config::SourceImage,
    journal: &JournalDisque,
    code: String,
) {
    // La couverture avant les images : elle manque aussi aux classeurs créés
    // AVANT que la correction du code de langue existe — `LOCH-JP` en est un.
    // La chercher à chaque ouverture les rattrape, et ne coûte rien quand
    // elle est déjà là (`telecharger` sort tout de suite).
    match runtime.block_on(ygo_app::couverture::telecharger(paths, client, &code)) {
        Ok(Some(_)) => fil.signaler(Evenement::Couverture { code: code.clone() }),
        Ok(None) => {}
        Err(e) => tracing::warn!(classeur = %code, erreur = %e, "couverture"),
    }

    let telechargeur = Telechargeur::nouveau(client, &paths.image_par_defaut());
    let cibles = match ygo_app::images::a_faire(paths, &code, source, &telechargeur) {
        Ok((cibles, _deja, _lignes)) => cibles,
        Err(e) => {
            // Classeur supprimé entre-temps, base illisible… : rien à
            // reprendre au prochain lancement non plus.
            journal.retirer(&code);
            fil.signaler(Evenement::Echec {
                code,
                raison: e.to_string(),
            });
            return;
        }
    };

    if cibles.is_empty() {
        journal.retirer(&code);
        fil.signaler(Evenement::Complet { code });
        return;
    }

    fil.signaler(Evenement::Debut {
        code: code.clone(),
        total: cibles.len(),
    });
    let bilan = runtime.block_on(telechargeur.toutes(&cibles, |faites, total| {
        fil.signaler(Evenement::Progression {
            code: code.clone(),
            faites,
            total,
        });
    }));
    journal.retirer(&code);
    ygo_app::replis::consigner(paths, &telechargeur.replis());
    fil.signaler(Evenement::Fin {
        code,
        reussies: bilan.reussies,
        echecs: bilan.echecs,
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn debut(code: &str, total: usize) -> Evenement {
        Evenement::Debut {
            code: code.to_owned(),
            total,
        }
    }
    fn progression(code: &str, faites: usize, total: usize) -> Evenement {
        Evenement::Progression {
            code: code.to_owned(),
            faites,
            total,
        }
    }
    fn fin(code: &str, reussies: usize, echecs: usize) -> Evenement {
        Evenement::Fin {
            code: code.to_owned(),
            reussies,
            echecs,
        }
    }

    fn classeurs(file: &VecDeque<TacheImages>) -> Vec<&str> {
        file.iter()
            .map(|t| match t {
                TacheImages::Classeur(c) | TacheImages::Apercus { code: c, .. } => c.as_str(),
            })
            .collect()
    }

    fn completer(code: &str, devant: bool) -> CommandeImages {
        CommandeImages::Completer {
            code: code.to_owned(),
            devant,
        }
    }

    /// Les classeurs créés prennent leur rang ; celui qu'on ouvre passe
    /// devant.
    #[test]
    fn un_classeur_ouvert_passe_devant_les_classeurs_crees() {
        let mut file = VecDeque::new();
        ranger(&mut file, completer("RA02", false));
        ranger(&mut file, completer("RA05", false));
        ranger(&mut file, completer("SDWD", true));
        assert_eq!(classeurs(&file), ["SDWD", "RA02", "RA05"]);
    }

    /// Un classeur déjà en file n'y entre pas deux fois : ouvert, il est
    /// déplacé en tête ; recréé, il garde sa place.
    #[test]
    fn un_classeur_deja_en_file_est_deplace_jamais_double() {
        let mut file = VecDeque::new();
        ranger(&mut file, completer("RA02", false));
        ranger(&mut file, completer("RA05", false));
        ranger(&mut file, completer("LDK2", false));

        ranger(&mut file, completer("RA05", false));
        assert_eq!(classeurs(&file), ["RA02", "RA05", "LDK2"], "rang gardé");

        ranger(&mut file, completer("LDK2", true));
        assert_eq!(
            classeurs(&file),
            ["LDK2", "RA02", "RA05"],
            "remonté, pas doublé"
        );
    }

    /// Les aperçus passent devant tout : l'utilisateur les attend à l'écran.
    #[test]
    fn les_apercus_passent_devant() {
        let mut file = VecDeque::new();
        ranger(&mut file, completer("RA02", false));
        ranger(
            &mut file,
            CommandeImages::Apercus {
                code: "LOCR-JP".to_owned(),
                cibles: Vec::new(),
            },
        );
        assert_eq!(classeurs(&file), ["LOCR-JP", "RA02"]);
    }

    /// Créé, un classeur sort des tâches en cours mais reste compté dans la
    /// file jusqu'à ses images — et seul ce moment-là autorise à le remonter.
    #[test]
    fn un_classeur_cree_attend_ses_images_dans_la_file() {
        let mut etat = Etat::default();
        etat.attendre("RA02");
        assert!(!etat.images_en_attente("RA02"), "pas encore créé");

        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 42,
        });
        assert!(!etat.images_en_attente("RA02"), "créé, artworks à venir");

        etat.appliquer(&Evenement::ImagesEnAttente {
            code: "RA02".to_owned(),
        });
        assert!(etat.images_en_attente("RA02"));
        assert!(etat.en_cours().is_empty(), "plus rien ne tourne pour lui");
        assert_eq!(etat.en_attente(), ["RA02"], "mais il attend toujours");

        etat.appliquer(&debut("RA02", 42));
        assert!(!etat.images_en_attente("RA02"), "ses images ont commencé");
        etat.appliquer(&fin("RA02", 42, 0));
        assert!(etat.en_attente().is_empty());
    }

    /// La mise à jour vers Yugipedia : ses classeurs entrent dans le lot,
    /// en attente d'images, et le bilan se lit en une ligne.
    #[test]
    fn les_classeurs_mis_a_jour_vers_yugipedia_attendent_leurs_images() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::ImagesTirage {
            lignes: 120,
            classeurs: 2,
            a_telecharger: 95,
        });
        assert!(etat.dernier().unwrap().contains("120 ligne(s)"));
        for code in ["RA02", "RA05"] {
            etat.appliquer(&Evenement::ImagesEnAttente {
                code: code.to_owned(),
            });
        }
        assert_eq!(etat.en_attente(), ["RA02", "RA05"]);
        assert_eq!(etat.lot(), Some((1, 2)));
        assert!(etat.images_en_attente("RA05"));

        etat.appliquer(&Evenement::ImagesTirage {
            lignes: 0,
            classeurs: 0,
            a_telecharger: 0,
        });
        assert!(etat.dernier().unwrap().contains("déjà à jour"));
    }

    /// Le bilan des numéros manquants se lit en une ligne, introuvables
    /// compris.
    #[test]
    fn le_bilan_des_numeros_manquants_dit_aussi_les_introuvables() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::NumerosAbsents {
            numeros: 1,
            lignes: 5,
            classeurs: 1,
            introuvables: 2,
        });
        let dernier = etat.dernier().unwrap();
        assert!(
            dernier.contains("1 numéro(s) ajouté(s) (5 ligne(s))"),
            "{dernier}"
        );
        assert!(dernier.contains("2 introuvable(s)"), "{dernier}");
        assert!(!etat.actif());
    }

    /// Le drapeau des travaux retombe avec sa garde.
    #[test]
    fn le_drapeau_des_travaux_retombe_avec_sa_garde() {
        let travaux = Travaux::default();
        assert!(!travaux.en_cours());
        {
            let _garde = travaux.ouvrir();
            assert!(travaux.clone().en_cours(), "vu depuis l'autre fil");
        }
        assert!(!travaux.en_cours());
        travaux.attendre_la_fin(); // rend la main aussitôt
    }

    #[test]
    fn une_passe_complete_ouvre_puis_referme_l_avancement() {
        let mut etat = Etat::default();
        assert!(!etat.actif());

        etat.appliquer(&debut("RA02", 42));
        assert!(etat.actif());
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 42
            }
        );

        etat.appliquer(&progression("RA02", 10, 42));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 10,
                total: 42
            }
        );

        etat.appliquer(&fin("RA02", 42, 0));
        assert!(!etat.actif(), "l'avancement se referme");
        assert_eq!(etat.dernier(), Some("RA02 — 42 image(s) récupérée(s)"));
    }

    /// Deux classeurs en même temps s'additionnent : la barre montre le
    /// travail total, pas celui du dernier arrivé.
    #[test]
    fn deux_passes_simultanees_s_additionnent() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 42));
        etat.appliquer(&debut("RA05", 8));
        etat.appliquer(&progression("RA02", 10, 42));
        etat.appliquer(&progression("RA05", 3, 8));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 13,
                total: 50
            }
        );
        assert_eq!(etat.en_cours().len(), 2);

        etat.appliquer(&fin("RA02", 42, 0));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 3,
                total: 8
            }
        );
        assert!(etat.actif(), "RA05 continue");
    }

    /// Un classeur déjà complet ne doit rien afficher du tout — ni barre, ni
    /// message. C'est le cas de huit classeurs sur neuf à chaque ouverture.
    #[test]
    fn un_classeur_complet_ne_montre_rien() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert!(!etat.actif());
        assert_eq!(etat.dernier(), None, "aucun message pour un non-événement");
        assert!(etat.a_rafraichir().is_empty(), "rien de neuf à relire");
    }

    #[test]
    fn un_echec_referme_l_avancement_et_le_dit() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 42));
        etat.appliquer(&Evenement::Echec {
            code: "RA02".to_owned(),
            raison: "base illisible".to_owned(),
        });
        assert!(!etat.actif());
        assert_eq!(etat.dernier(), Some("RA02 — base illisible"));
    }

    #[test]
    fn le_message_distingue_les_indisponibles() {
        let mut etat = Etat::default();
        etat.appliquer(&fin("RA02", 40, 2));
        assert_eq!(
            etat.dernier(),
            Some("RA02 — 40 récupérée(s), 2 indisponible(s)")
        );
    }

    /// Le rafraîchissement se demande **à chaque image**, pas seulement à la
    /// fin : un classeur de cinq cents images resterait troué jusqu'au bout.
    #[test]
    fn chaque_image_recue_demande_un_rafraichissement() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 3));
        etat.appliquer(&progression("RA02", 1, 3));
        etat.appliquer(&progression("RA02", 2, 3));
        etat.appliquer(&fin("RA02", 3, 0));
        assert_eq!(etat.a_rafraichir().len(), 3, "deux images plus la fin");
    }

    /// La liste se consomme : le cache n'oublie ses absents qu'une fois par
    /// arrivée, pas à chaque image dessinée.
    #[test]
    fn la_liste_a_rafraichir_se_consomme() {
        let mut etat = Etat::default();
        etat.appliquer(&progression("RA02", 1, 3));
        assert_eq!(etat.a_rafraichir(), ["RA02"]);
        assert!(etat.a_rafraichir().is_empty(), "une seule fois");
    }

    /// Une création annonce son début sans total — on ne sait pas encore
    /// combien de lignes le set fera — puis se conclut par `Cree`.
    #[test]
    fn une_creation_ouvre_un_avancement_sans_total_puis_se_conclut() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        assert!(etat.actif());
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 0
            }
        );

        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert_eq!(etat.dernier(), Some("RA02 — classeur créé, 553 ligne(s)"));
    }

    /// L'accueil ne se recharge qu'une fois par création : la liste se
    /// **consomme**, sinon la lecture des neuf classeurs repartirait à chaque
    /// image dessinée.
    #[test]
    fn les_creations_se_consomment() {
        let mut etat = Etat::default();
        assert!(etat.creations().is_empty());

        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert_eq!(etat.creations(), ["RA02"]);
        assert!(etat.creations().is_empty(), "une seule fois");
    }

    /// L'étape est nommée à chaque instant : sans elle, la bande annonçait
    /// « Images » pendant que le classeur s'écrivait encore.
    #[test]
    fn l_etape_suit_le_travail_reel() {
        let mut etat = Etat::default();
        assert_eq!(etat.etape("RA02"), None);

        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), Some(Etape::Creation));

        etat.appliquer(&Evenement::ArtworksEnCours {
            code: "RA02".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), Some(Etape::Artworks));

        etat.appliquer(&debut("RA02", 42));
        assert_eq!(etat.etape("RA02"), Some(Etape::Images));

        etat.appliquer(&fin("RA02", 42, 0));
        assert_eq!(etat.etape("RA02"), None, "l'étape s'efface avec la tâche");
    }

    /// Les trois étapes ont des libellés distincts — sinon la bande dirait la
    /// même chose du début à la fin.
    #[test]
    fn les_trois_etapes_ont_des_libelles_distincts() {
        let mut libelles = [
            Etape::Creation.libelle(),
            Etape::Artworks.libelle(),
            Etape::Images.libelle(),
        ];
        libelles.sort_unstable();
        let avant = libelles.len();
        let mut sans_doublon = libelles.to_vec();
        sans_doublon.dedup();
        assert_eq!(sans_doublon.len(), avant);
    }

    /// Un échec ou un classeur complet efface aussi l'étape : la bande ne doit
    /// pas garder un libellé orphelin.
    #[test]
    fn l_etape_s_efface_aussi_sur_echec_et_sur_complet() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        etat.appliquer(&Evenement::Echec {
            code: "RA02".to_owned(),
            raison: "x".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), None);

        etat.appliquer(&debut("RA05", 3));
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert_eq!(etat.etape("RA05"), None);
    }

    /// La création enchaîne sur les images du même classeur : l'avancement
    /// passe du total inconnu au total réel sans se refermer entre-temps.
    #[test]
    fn la_creation_enchaine_sur_les_images_sans_se_refermer() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert!(etat.actif(), "la tâche continue vers les images");

        etat.appliquer(&debut("RA02", 123));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 123
            }
        );
        etat.appliquer(&fin("RA02", 123, 0));
        assert!(!etat.actif());
    }

    #[test]
    fn chaque_evenement_nomme_son_classeur_et_dit_s_il_termine() {
        assert_eq!(debut("RA02", 1).code(), "RA02");
        assert!(!debut("RA02", 1).termine());
        assert!(!progression("RA02", 1, 2).termine());
        assert!(fin("RA02", 1, 0).termine());
        assert!(Evenement::Complet {
            code: "RA02".to_owned()
        }
        .termine());
        assert!(Evenement::Echec {
            code: "RA02".to_owned(),
            raison: String::new()
        }
        .termine());
        // Une création n'est pas une fin : les images suivent.
        assert!(!Evenement::CreationEnCours {
            code: "RA02".to_owned()
        }
        .termine());
        assert!(!Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 1
        }
        .termine());
    }

    /// La mise à jour de la base se comporte comme une tâche parmi les
    /// autres : elle ouvre l'avancement, le tient, et le referme.
    #[test]
    fn la_mise_a_jour_de_la_base_ouvre_puis_referme_l_avancement() {
        let mut etat = Etat::default();
        assert!(!etat.base_active());

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        assert!(etat.actif());
        assert!(etat.base_active());
        assert_eq!(
            etat.etape(CODE_BASE),
            Some(Etape::Base(PhaseBase::Telechargement))
        );

        etat.appliquer(&progression(CODE_BASE, 1_048_576, 170_000_000));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 1_048_576,
                total: 170_000_000
            }
        );

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Ecriture,
        });
        assert_eq!(
            etat.etape(CODE_BASE),
            Some(Etape::Base(PhaseBase::Ecriture)),
            "le libellé suit la phase"
        );
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 0
            },
            "une phase sans total connu remet la barre à l'animation"
        );

        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 13_500,
            tirages: 320_000,
            version: Some("146.68".to_owned()),
        });
        assert!(!etat.actif());
        assert!(!etat.base_active());
        let dernier = etat.dernier().unwrap_or_default();
        assert!(dernier.contains("146.68"), "{dernier}");
    }

    /// La base reconstruite est une nouveauté pour l'accueil : tous les
    /// classeurs la relisent.
    #[test]
    fn une_base_reconstruite_fait_recharger_l_accueil() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 1,
            tirages: 1,
            version: None,
        });
        assert_eq!(etat.creations(), vec![CODE_BASE.to_owned()]);
    }

    /// Sans version, on le dit — plutôt que de laisser croire à un succès
    /// complet.
    #[test]
    fn une_version_manquante_est_annoncee() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 1,
            tirages: 1,
            version: None,
        });
        let dernier = etat.dernier().unwrap_or_default();
        assert!(dernier.contains("version"), "{dernier}");
    }

    /// Un échec de mise à jour se referme comme les autres, sous le même nom.
    #[test]
    fn un_echec_de_mise_a_jour_referme_l_avancement() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        etat.appliquer(&Evenement::Echec {
            code: CODE_BASE.to_owned(),
            raison: "GitHub injoignable".to_owned(),
        });
        assert!(!etat.base_active());
        assert!(!etat.actif());
    }

    /// Les deux événements de base répondent `code()` sans qu'on ait à le
    /// leur passer — c'est ce qui permet au service de reconnaître la fin.
    #[test]
    fn les_evenements_de_base_se_nomment_seuls() {
        assert_eq!(
            Evenement::BaseEnCours {
                phase: PhaseBase::Analyse
            }
            .code(),
            CODE_BASE
        );
        assert!(!Evenement::BaseEnCours {
            phase: PhaseBase::Analyse
        }
        .termine());
        assert!(Evenement::BaseTerminee {
            cartes: 0,
            tirages: 0,
            version: None
        }
        .termine());
    }

    /// Chaque phase a son libellé, et aucun n'est vide : la bande de
    /// progression les affiche tels quels.
    #[test]
    fn les_phases_ont_des_libelles_distincts() {
        let phases = [
            PhaseBase::Telechargement,
            PhaseBase::Catalogue,
            PhaseBase::Analyse,
            PhaseBase::Artworks,
            PhaseBase::Ecriture,
        ];
        let mut vus = std::collections::HashSet::new();
        for phase in phases {
            let libelle = Etape::Base(phase).libelle();
            assert!(!libelle.is_empty());
            assert!(vus.insert(libelle), "libellé répété : {libelle}");
        }
    }

    fn a_faire(locale: Option<&str>, distante: &str) -> Evenement {
        Evenement::Version {
            verdict: ygo_app::maj::Verdict::AFaire {
                locale: locale.map(ToOwned::to_owned),
                distante: distante.to_owned(),
            },
        }
    }

    /// Le bandeau ne paraît que sur un verdict qui dit qu'il y a à faire.
    #[test]
    fn le_bandeau_suit_le_verdict() {
        let mut etat = Etat::default();
        assert!(
            !etat.maj_disponible(),
            "rien tant que le serveur n'a rien dit"
        );

        etat.appliquer(&Evenement::Version {
            verdict: ygo_app::maj::Verdict::AJour {
                version: "146.68".to_owned(),
            },
        });
        assert!(!etat.maj_disponible(), "à jour ne se signale pas");

        etat.appliquer(&Evenement::Version {
            verdict: ygo_app::maj::Verdict::Indecidable {
                raison: "serveur injoignable".to_owned(),
            },
        });
        assert!(
            !etat.maj_disponible(),
            "indécidable ne se signale pas non plus"
        );

        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        assert!(etat.maj_disponible());
    }

    /// Annoncer « disponible » pendant que la mise à jour s'installe serait
    /// absurde : le bandeau se tait dès que le travail commence.
    #[test]
    fn le_bandeau_se_tait_pendant_la_mise_a_jour() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        assert!(etat.maj_disponible());

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        assert!(!etat.maj_disponible(), "pas pendant le travail");
    }

    /// Écarter le bandeau ne change pas le verdict — c'est son affichage qui
    /// se tait.
    #[test]
    fn ecarter_le_bandeau_garde_le_verdict() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(None, "147.01"));
        etat.ecarter_maj();
        assert!(!etat.maj_disponible());
        assert!(
            matches!(etat.version(), Some(ygo_app::maj::Verdict::AFaire { .. })),
            "le contrôle reste vrai"
        );
    }

    /// Après une reconstruction, le verdict d'avant ne veut plus rien dire :
    /// il est effacé, pas deviné. Le fil en redemande un au serveur.
    #[test]
    fn une_mise_a_jour_efface_le_verdict_precedent() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        etat.ecarter_maj();

        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 13_500,
            tirages: 320_000,
            version: Some("147.01".to_owned()),
        });
        assert_eq!(etat.version(), None, "effacé, pas déduit");
        assert!(!etat.maj_disponible());

        // Et le nouveau verdict repart d'une ardoise propre : écarter le
        // bandeau une fois ne le condamne pas pour la session entière.
        etat.appliquer(&a_faire(Some("147.01"), "147.02"));
        assert!(etat.maj_disponible());
    }

    /// Le contrôle de version se nomme comme la base, et ne termine rien —
    /// sans quoi il libérerait la garde anti double-lancement.
    #[test]
    fn le_controle_de_version_ne_termine_rien() {
        let evenement = a_faire(None, "147.01");
        assert_eq!(evenement.code(), CODE_BASE);
        assert!(!evenement.termine());
    }

    /// Le retour d'usage du 2026-09-05 : trois classeurs demandés d'un coup,
    /// et la bande n'en annonçait qu'un.
    #[test]
    fn la_file_compte_ce_qui_attend_son_tour() {
        let mut etat = Etat::default();
        assert_eq!(etat.lot(), None, "rien demandé, rien à dire");

        for code in ["RA05", "LOCR-JP", "LOCH-JP"] {
            etat.attendre(code);
        }
        assert_eq!(etat.lot(), Some((1, 3)));

        // Le fil n'en traite qu'un : les deux autres attendent.
        etat.appliquer(&debut("RA05", 156));
        assert_eq!(etat.lot(), Some((1, 3)));
        assert_eq!(etat.en_attente(), vec!["LOCH-JP", "LOCR-JP"]);

        etat.appliquer(&fin("RA05", 156, 0));
        assert_eq!(etat.lot(), Some((2, 3)), "le rang avance");

        etat.appliquer(&debut("LOCH-JP", 300));
        etat.appliquer(&fin("LOCH-JP", 300, 0));
        assert_eq!(etat.lot(), Some((3, 3)));

        etat.appliquer(&debut("LOCR-JP", 10));
        etat.appliquer(&fin("LOCR-JP", 10, 0));
        assert_eq!(etat.lot(), None, "le lot fini ne s'affiche plus");
    }

    /// « 1/1 » n'apprend rien que la bande ne dise déjà.
    #[test]
    fn une_tache_seule_ne_montre_pas_de_rang() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        assert_eq!(etat.lot(), None);
        etat.appliquer(&debut("RA05", 42));
        assert_eq!(etat.lot(), None);
    }

    /// Un classeur demandé une heure après le lot précédent ne doit pas
    /// s'annoncer « 7/7 » : une file drainée ouvre un lot neuf.
    #[test]
    fn une_file_videe_ouvre_un_lot_neuf() {
        let mut etat = Etat::default();
        etat.attendre("A");
        etat.attendre("B");
        etat.appliquer(&fin("A", 1, 0));
        etat.appliquer(&fin("B", 1, 0));
        assert_eq!(etat.lot(), None);

        etat.attendre("C");
        etat.attendre("D");
        assert_eq!(etat.lot(), Some((1, 2)), "et non (3, 4)");
    }

    /// Un classeur complet n'a rien à télécharger : il sort de la file comme
    /// les autres, sinon elle ne se viderait jamais.
    #[test]
    fn un_classeur_complet_sort_de_la_file() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        etat.attendre("VASM");
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert_eq!(etat.lot(), Some((2, 2)));
        assert_eq!(etat.en_attente(), vec!["VASM"]);
    }

    /// Un échec aussi : sans quoi une passe ratée bloquerait le compteur.
    #[test]
    fn un_echec_sort_de_la_file() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        etat.attendre("VASM");
        etat.appliquer(&Evenement::Echec {
            code: "RA05".to_owned(),
            raison: "réseau".to_owned(),
        });
        assert_eq!(etat.lot(), Some((2, 2)));
    }
}
