---
type: Architecture
title: "Distribution et mises à jour du helper"
description: "How the native helper is built, packaged with Velopack, published to the public sprkl/faulk-release repository, served through get.faulk.app redirects on Vercel, and updated on the user's machine (stable and beta channels, deltas, signing to come)."
status: draft
---

# Distribution et mises à jour du helper

Ce document décrit la chaîne qui va du dépôt au helper installé sur un poste, puis la manière dont ce helper se met à jour. Le modèle est celui de lebureau-desktop : le code reste privé, les binaires sont publics dans un dépôt de releases à part, et le helper ne lit que notre domaine, qui redirige vers ces releases.

> **État (14 octobre 2026).** Le module `crates/helper/src/updater.rs`, la CI `.github/workflows/release-helper.yml` et les redirections `web/vercel.json` sont écrits et testés, et `main.rs` appelle l'updater (voir [Le câblage dans `main.rs`](#le-câblage-dans-mainrs)). La chaîne complète (installation de la 0.1.0, mise à jour vers la 0.1.1 en différentiel, passage en bêta, application au démarrage et sur demande, désinstallation) a été éprouvée sur un poste Windows avec un flux local, puis refaite avec le vrai helper câblé (voir [Éprouver la chaîne en local](#éprouver-la-chaîne-en-local)).
>
> Reste à faire, dans cet ordre : le secret `RELEASES_TOKEN`, le domaine `get.faulk.app` sur Vercel, une première version publiée par la CI, les deux entrées de mise à jour du menu de la zone de notification (leurs textes attendent la validation du fondateur), puis la signature.

## L'outil : Velopack

Velopack (crate `velopack` et outil `vpk`, tous deux en **1.2.161**, versions à garder identiques) produit à partir du binaire de `cargo build` :

- un installeur Windows `FaulkHelper-win-Setup.exe` (installation par utilisateur, sans droits d'administrateur, dans `%LocalAppData%\FaulkHelper`) et une version portable ;
- un paquet complet `FaulkHelper-<version>-full.nupkg` et, quand la version précédente est disponible au moment du build, un paquet **différentiel** `FaulkHelper-<version>-delta.nupkg` (mesuré en local : 231 Ko contre 4,3 Mo pour le paquet complet) ;
- le flux `releases.win.json` (`releases.linux.json`, `releases.osx.json` sur les autres systèmes), un simple fichier JSON servi par n'importe quel hébergement statique.

Côté helper, le SDK Rust vérifie le flux, télécharge (différentiel d'abord, complet en repli), contrôle la taille et l'empreinte SHA du paquet, puis applique la mise à jour au prochain démarrage ou sur demande, et relance le helper.

Les alternatives examinées :

| Outil | Ce qui l'écarte |
|---|---|
| cargo-dist + axoupdater | Bons installeurs (scripts shell et PowerShell, MSI), mais axoupdater ne lit que GitHub Releases ou Axo Releases (`ReleaseSourceType::{GitHub, Axo}`) : pas notre domaine. Pas de différentiel, pas d'application au redémarrage. |
| self_update | Remplace le binaire en place depuis GitHub, GitLab, S3 ou un manifeste statique. Pas d'installeur, pas de différentiel, rien pour orchestrer l'application et la relance. |

Velopack est le seul des trois qui couvre installeur, différentiels, hébergement statique et application au redémarrage, et il expose le trait `UpdateSource`, grâce auquel les canaux et les URLs sont notre code, testable sans réseau.

## La distribution : un dépôt GitHub public

Une release de `sprkl/faulk`, dépôt privé, exigerait un jeton côté client. Les fichiers sont donc publiés une seconde fois dans les releases du dépôt **public** [`sprkl/faulk-release`](https://github.com/sprkl/faulk-release), qui ne contient qu'un README.

Le montage est asymétrique :

- **en publication**, la CI crée la release dans `sprkl/faulk` puis la recrée dans `sprkl/faulk-release` : mêmes fichiers, même tag `helper-v<version>`, mêmes drapeaux. `GITHUB_TOKEN` n'écrit que dans son propre dépôt : il faut le secret `RELEASES_TOKEN` ;
- **en consommation**, le helper ne lit jamais `github.com` directement : il lit `https://get.faulk.app/helper/...`, que Vercel redirige vers les releases publiques. L'URL inscrite dans le helper est la nôtre : l'hébergement peut changer sans rendre muet un helper déjà installé.

## Les redirections de `get.faulk.app`

Elles vivent dans `web/vercel.json`, lu par le projet Vercel qui déploie `web/`. Chaque règle porte la condition `has: host = get.faulk.app` : `faulk.app` lui-même n'est pas touché. Toutes sont temporaires (`permanent: false`), pour pouvoir changer d'hébergement.

| Demandé à `get.faulk.app` | Redirigé vers (`github.com/sprkl/faulk-release/releases/...`) | Pourquoi |
|---|---|---|
| `/helper/stable/releases.<os>.json` | `latest/download/releases.<os>.json` | `latest` est la dernière release ni brouillon ni pré-version : une bêta n'atteint jamais le canal stable. |
| `/helper/beta/releases.<os>.json` | `download/helper-beta/releases.<os>.json` | `helper-beta` est une pré-version « tournante » dont la CI remplace le flux à chaque bêta publiée. Elle n'est jamais `latest`. |
| `/helper/packages/FaulkHelper-<version>-<full\|delta>.nupkg` | `download/helper-v<version>/<fichier>` | Le paquet est cherché dans la release de **sa** version. Le flux d'une version liste aussi le paquet complet de la précédente, qui n'existe que dans sa propre release. |
| `/helper/windows` | `latest/download/FaulkHelper-win-Setup.exe` | Le lien de téléchargement stable à donner. |
| tout le reste | `latest` (la page de la dernière release) | Sans cette règle, `get.faulk.app` servirait l'application web. |

Le test `crates/helper/src/updater/tests.rs` lit `web/vercel.json` et vérifie que chaque URL construite par le helper aboutit au bon fichier : le client et les redirections ne peuvent pas diverger sans qu'un test casse. Sa limite : il traduit lui-même la syntaxe des sources (sous-ensemble de path-to-regexp), il ne remplace donc pas la vérification après déploiement (voir [Ce que le fondateur doit faire](#ce-que-le-fondateur-doit-faire)).

## Les canaux

- **Stable** : le helper lit le flux stable seulement, et en retire toute pré-version qui s'y trouverait par erreur. Deux barrières, donc : `releases/latest` côté GitHub, le filtre côté client.
- **Bêta** : le helper lit les deux flux et prend la version la plus récente. Un utilisateur bêta reçoit donc aussi une version stable plus récente que sa bêta. Si le flux bêta n'existe pas encore (aucune bêta publiée), le flux stable suffit.
- **Choix du canal** : celui de la version installée (une `0.3.0-beta.1` est en bêta). La variable d'environnement `FAULK_UPDATE_CHANNEL=stable|beta` le force. Conséquence à connaître : un utilisateur bêta qui reçoit une version stable repasse en stable. **Décidé par le fondateur le 14 octobre 2026** : le canal suit la version installée, sans réglage ni mémoire du canal (une installation stable reçoit les versions stables, une installation de pré-version reçoit les pré-versions).
- **Différentiels** : seulement sur le canal stable. Un différentiel ne s'applique qu'à la version à partir de laquelle il a été construit, et la CI le construit toujours à partir de la dernière version stable. Un poste bêta (bêtas successives, puis une stable construite sur une autre base) ne suit pas cette ligne : le canal bêta ignore les différentiels et télécharge les paquets complets.

## Côté helper : `crates/helper/src/updater.rs`

- `updater::startup()` : premier appel de `main`, avant la lecture des arguments. L'installeur et l'outil de mise à jour lancent l'exécutable avec des arguments `--veloapp-*` et attendent qu'il quitte aussitôt. Il applique aussi une mise à jour téléchargée lors d'une exécution précédente, puis relance le helper sur la nouvelle version, **sauf si un autre helper tourne déjà** : appliquer remplace les fichiers de l'installation et couperait le son de l'autre helper. Pour le savoir, chaque helper tient toute sa vie un verrou partagé sur `<config>/faulk/helper.lock` ; un helper est seul s'il peut prendre ce verrou en exclusif, quel que soit l'ordre de démarrage des autres. Un helper seul garde le verrou exclusif le temps de décider (et d'appliquer, ce qui le fait quitter) ; un helper qui démarre pendant ce temps attend d'obtenir le verrou partagé (il le signale toutes les dix secondes) ; il ne tourne pas sans lui (sauf erreur d'ouverture, voir plus bas), sinon les autres ne le verraient pas. Limites connues :
  - **la fenêtre de l'application** : le verrou est libéré quand le helper qui applique quitte, alors que `Update.exe` remplace les fichiers *après* cette sortie. Un helper lancé pendant ces quelques secondes se croit seul ; `Update.exe` peut alors l'arrêter ou remplacer ses fichiers, puis relancer l'ancien, qui trouve son port pris et ouvre la page. Le second helper vient de démarrer, il ne jouait pas encore : la piste pour fermer ce cas est d'attendre aussi le verrou de Velopack (`packages/.velopack_lock`), à condition de vérifier qu'`Update.exe` le tient pendant toute l'application ;
  - **un verrou impossible à ouvrir** (erreur autre qu'une attente, par exemple un antivirus qui tient `helper.lock`) : le helper démarre quand même, sans verrou, plutôt que de refuser de démarrer ; un autre helper seul pourrait alors appliquer sous lui ;
  - **l'attente sans fin** : un helper qui démarre attend tant qu'un autre tient le verrou en exclusif. Un processus mort le libère ; seul un helper bloqué vivant pendant sa décision ferait attendre les suivants, sans message visible (la ligne d'attente part avant l'ouverture du journal). Chaque changement de mode passe par un second verrou bref (`helper.gate`), si bien que deux helpers lancés ensemble ne se croient jamais seuls tous les deux.
- `Updater::start(Config::from_env())` : lance un thread `faulk-updater`. Vérification au démarrage, puis toutes les six heures. Une version trouvée est téléchargée en arrière-plan. Hors installation (`cargo run`), l'updater est désactivé et ne touche pas le réseau.
- `Updater::check_now()` : vérification immédiate, sans attendre ; les demandes répétées pendant une vérification se fondent en une seule.
- `Updater::shutdown()` : arrête le thread et attend au plus la fin de la vérification ou du téléchargement en cours ; une vérification demandée entre-temps est abandonnée.
- `Updater::status()` : `Disabled`, `Idle`, `Checking`, `UpToDate`, `Downloading { percent }`, `Ready`, `Failed { error }`. Les erreurs sont classées : réseau, flux absent (404 sur le flux), paquet absent (404 sur un paquet listé : release incomplète ou redirection cassée), flux invalide, paquet corrompu, autre. Un échec est retenté à la vérification suivante. Une version déjà téléchargée reste applicable après un échec.
- `Updater::apply_and_restart()` : applique la version téléchargée et relance le helper avec les mêmes arguments. Refusé, avec une erreur explicite, tant qu'un autre helper tourne (même verrou qu'au démarrage, pris en exclusif le temps de l'application), et pendant une vérification ou un téléchargement du thread de mise à jour (Velopack appliquerait depuis un dossier de paquets en cours d'écriture). Le processus quitte : la sortie audio s'arrête le temps de la relance, c'est voulu et c'est l'utilisateur qui le demande.
- **Audio** : l'updater ne s'exécute jamais sur le thread audio et ne partage aucun état avec lui. Ses seuls effets sur le son sont les deux sorties de processus ci-dessus.
- **Journal** : les lignes `update: ...` et les avertissements et erreurs de Velopack (cible `velopack*` seulement, par un petit journal `log` installé par `startup()`) vont au journal du helper (`crates/helper/src/log.rs` : le fichier `<config>/faulk/logs/helper.log` et la sortie d'erreur). `startup()` s'exécute avant l'ouverture de ce fichier : ses propres lignes (« another helper is running… ») ne vont qu'à la sortie d'erreur, absente d'un build de release sous Windows ; l'application elle-même est faite par `Update.exe`, qui tient son journal (`%LocalAppData%\velopack\velopack.log`). Le journal `log` vaut pour tout le processus : si un autre journal `log` est installé avant, celui-ci ne l'est pas, sans erreur ; s'il l'est après, l'autre doit tolérer l'échec de `set_logger` (`env_logger::init()` paniquerait, `try_init()` non).
- **Vie privée** : chaque lecture du flux porte `?localVersion=<version>&id=FaulkHelper&stagingId=<identifiant>`. Ce paramètre est ajouté par Velopack ; `stagingId` est un identifiant aléatoire propre à l'installation, prévu pour les déploiements progressifs. Il arrive à Vercel et, par la redirection, à GitHub.
- **Tests de pointage** : la variable `FAULK_UPDATE_URL` remplace `https://get.faulk.app/helper` par une adresse `https://` (n'importe quel hôte) ou `http://` vers la boucle locale seulement (`127.0.0.1`, `localhost`, `[::1]`), sans requête ni fragment (les chemins du flux et des paquets y sont ajoutés). Les paquets ne sont vérifiés que contre le flux servi par cette même adresse : qui peut écrire l'environnement de l'utilisateur choisit le code installé. C'est accepté, car qui peut écrire cet environnement peut déjà exécuter du code sous ce compte ; la signature de code (plus bas) fermera ce point.

### Le câblage dans `main.rs`

`main.rs` appelle l'updater depuis le 14 octobre 2026 :

- `helper::updater::startup()` est la première instruction de `main`, avant la lecture des arguments, la prise du port (un helper par port, #12) et l'ouverture du journal. Un second helper lancé sur un port déjà servi passe donc d'abord par `startup()` : il voit l'autre helper au verrou d'instance et n'applique rien, puis ouvre la page et quitte.
- `Updater::start(Config::from_env())` est appelé une fois le serveur lancé ; l'updater vit aussi longtemps que le helper et n'est pas attendu à la sortie (`Quitter`) : une vérification ou un téléchargement en cours est abandonné avec le processus. Hors installation, le journal dit `update: disabled (...)`.
- **Désinstallation** : `startup()` inscrit un crochet Velopack `before_uninstall` qui retire la valeur « Lancer au démarrage de Windows » (clé `Run`, valeur `FaulkHelper`) quand elle lance l'exécutable désinstallé ; une valeur qui lance une autre copie (un build de développement) n'est pas touchée. Le crochet ouvre lui-même le journal pour y écrire ce qu'il a fait (`uninstall: start with Windows removed`). Sans cela, la désinstallation laissait une entrée de démarrage vers un fichier supprimé.

Ce qui manque : les deux entrées du menu de la zone de notification, « Rechercher des mises à jour » (`updater.check_now()`) et « Redémarrer pour mettre à jour » (visible quand `status()` vaut `Ready`, `updater.apply_and_restart()`). Ce sont de nouveaux textes d'interface, en anglais et en français : ils attendent la validation du fondateur. En attendant, une mise à jour téléchargée s'applique au démarrage suivant du helper, et l'application sur demande, écrite et testée, n'est pas encore accessible à l'utilisateur.

Sans ces appels, un installeur produit par la CI installerait un helper qui ne répond pas aux arguments `--veloapp-*` et ne se met jamais à jour. `vpk` ne le détecte pas : la CI le vérifie elle-même et **refuse de publier** une version sans ces appels (un build d'essai passe, avec un avertissement). Elle lance le binaire construit comme le ferait l'installeur (`timeout 30 faulk-helper.exe --veloapp-install 0.0.1`) : un helper câblé répond aussitôt par 0 ; un helper non câblé refuse l'argument inconnu (code 1, et dans un build de release une boîte d'erreur qui le bloque jusqu'à l'expiration du délai). `Updater::start` ne s'observe pas hors installation : la CI cherche son appel, hors commentaire, dans `main.rs`.

Ces actions de la zone de notification concernent le helper lui-même, pas le projet musical. Elles ne passent pas par `crates/commands`. Si l'interface web doit un jour les proposer, la question du passage par une commande sera à trancher à ce moment-là.

## Publier une version

`.github/workflows/release-helper.yml`, en quatre jobs :

1. **version** : depuis le tag `helper-v<version>` ou la saisie d'un lancement manuel. Format vérifié (`X.Y.Z` ou `X.Y.Z-suffixe`), un suffixe fait une pré-version. Un lancement manuel avec version ne publie que depuis `main` (stable) ou `main`/`develop` (pré-version). Même règle pour un tag poussé : un tag stable doit être sur un commit de `main`, un tag de pré-version sur `main` ou `develop`, sinon le run échoue avant de construire.
2. **windows** (obligatoire) : tests du helper, build `--release`, téléchargement de la dernière release stable publique (pour le différentiel, sauf pré-version et build d'essai), `vpk pack --runtime win-x64`, artefacts téléchargeables quatorze jours.
3. **unix** (macOS arm64 et Linux x64, facultatifs, `continue-on-error`) : build et empaquetage pour mémoire, **jamais publiés**, ni signés ni testés.
4. **release** (seulement avec une version) : refuse un helper non câblé, crée la release dans `sprkl/faulk` (notes générées), puis la copie publique dans `sprkl/faulk-release` avec des notes fixes (les notes générées citent les pull requests privées). Sans `RELEASES_TOKEN`, la copie est sautée avec un avis explicite dans le résumé du run ; une fois le secret créé, relancer le job suffit : une release qui existe déjà voit ses fichiers remplacés au lieu de faire échouer le run. Une pré-version met aussi à jour le flux de la release `helper-beta`, sauf en brouillon (ses paquets ne seraient pas téléchargeables).

Une version ne devient `latest` que si elle est plus récente que l'actuelle : relancer un ancien tag ne fait pas reculer le canal stable.

Trois façons de lancer :

- la voie normale : `git tag helper-v0.2.0 && git push origin helper-v0.2.0` ;
- sans tag : `gh workflow run release-helper.yml --ref main -f version=0.2.0` ;
- un build d'essai : `gh workflow run release-helper.yml` sans version, ou toute pull request qui modifie ce workflow, l'updater, `main.rs`, `lib.rs`, `crates/helper/Cargo.toml` ou `Cargo.lock` (le câblage et la version de Velopack passent ainsi la garde dès la PR).

`gh workflow run` ne fonctionne qu'une fois le workflow présent sur la branche par défaut (`develop`).

## Compatibilité avec la page

Le helper se met désormais à jour à son rythme, indépendamment du déploiement de `faulk.app`.

**Règle, décidée par le fondateur le 14 octobre 2026** : le helper et la page doivent avoir exactement la même `PROTOCOL_VERSION` (`crates/protocol`). La page refuse déjà tout helper d'une autre version (`web/src/lib/engine/helper-client.ts`, état `incompatible` de la session). Pas de plage de versions acceptées.

Conséquence : **un changement de protocole se livre d'un bloc**, la version du helper publiée et la page déployée ensemble. Tant que l'un des deux manque, le mode Boost est coupé pour tous les utilisateurs dont le helper et la page ne sont plus d'accord : la page affiche `helper.incompatible` (« This helper speaks another protocol version ({v}). Update it. », « Ce helper parle une autre version du protocole ({v}). Mets-le à jour. ») et reste en mode Web. Ce message invite à mettre à jour le helper, ce qui est juste quand la page est la plus récente ; quand le helper est le plus récent (page pas encore déployée, ou ancienne page en cache), c'est la page qu'il faut recharger. Le message ne distingue pas encore les deux cas : une évolution du texte est proposée au fondateur.

## La signature, à venir

| | Installation non signée | Mise à jour non signée |
|---|---|---|
| **Windows** | SmartScreen avertit, l'installation passe | fonctionne (éprouvé en local) |
| **macOS** | Gatekeeper bloque, contournement manuel | à éprouver ; Apple exige une app signée et notarisée pour une distribution normale |
| **Linux** (AppImage) | rien à signer | non éprouvé |

- **Windows** : Azure Artifact Signing (ex-« Trusted Signing »), de l'ordre de 10 $ par mois, sans clé ni jeton matériel à gérer. `vpk pack --azureTrustedSignFile metadata.json` signe l'installeur et les binaires, après un `azure/login` dans le job. La place est marquée dans le workflow. SmartScreen n'est pas supprimé pour autant : la réputation s'accumule au fil des versions signées.
- **macOS** : compte Apple Developer (99 $ par an), signature « Developer ID » et notarisation (`vpk pack` accepte `--signAppIdentity`, `--signInstallIdentity` et `--notaryProfile` sur un runner macOS). À faire avant la première version macOS publiée : passer de non signé à signé change l'identité de l'app.
- **Linux** : rien.

## macOS et Linux : ce qui manque

Le job `unix` construit et empaquette sans publier. Avant de publier : choisir les architectures (arm64 et x64 sur macOS), signer et notariser macOS, éprouver installation et mise à jour sur un poste de chaque système, ajouter `releases.osx.json` et `releases.linux.json` à l'upload de la release (les redirections les acceptent déjà), et un lien de téléchargement par système (`/helper/macos`, `/helper/linux`).

## Éprouver la chaîne en local

Ce qui a été fait le 14 octobre 2026 sur Windows 11, d'abord avec un `main.rs` temporairement câblé (non commité) :

1. `cargo build -p helper --release`, copie de `faulk-helper.exe` seul dans un dossier `app/` ;
2. `vpk pack --packId FaulkHelper --runtime win-x64 --packDir app --mainExe faulk-helper.exe --packVersion 0.1.0 --outputDir R`, copie de `FaulkHelper-win-Setup.exe`, puis la même commande en `0.1.1` (différentiel construit depuis la 0.1.0) et en `0.2.0-beta.1` dans un autre dossier ;
3. un dossier servi par `python -m http.server 8765 --bind 127.0.0.1` qui reproduit les redirections : `stable/releases.win.json`, `beta/releases.win.json`, `packages/*.nupkg` ;
4. installation silencieuse de la 0.1.0 (`Setup.exe --silent`), lancement avec `FAULK_UPDATE_URL=http://127.0.0.1:8765` : le helper lit le flux stable seulement (pas la bêta), télécharge le différentiel 0.1.1, passe `Ready` ;
5. second lancement : la 0.1.1 est appliquée au démarrage, le helper relancé avec les mêmes arguments ;
6. `FAULK_UPDATE_CHANNEL=beta` : la `0.2.0-beta.1` est téléchargée en paquet complet ; un second helper lancé pendant ce temps **n'applique pas** la mise à jour (le premier continue de jouer) ;
7. une `0.2.0-beta.2` ajoutée au flux bêta : appliquée sur demande (`apply_and_restart`), le helper relancé en `0.2.0-beta.2` ;
8. désinstallation : `%LocalAppData%\FaulkHelper\Update.exe --uninstall --silent` retire le dossier et les raccourcis.

Après la revue de livraison, le verrou est devenu partagé (chaque helper le tient) et l'application sur demande le vérifie aussi. Ce nouveau verrou a été éprouvé le même jour avec un petit exécutable de test hors dépôt, qui fait les mêmes appels que `main.rs` devra faire (`startup()`, `Updater::start`, `apply_and_restart`), sans audio, empaqueté sous le même identifiant `FaulkHelper` :

1. A (0.1.0) télécharge la 0.1.1 ; B démarre à côté (« another helper is running ») ; A quitte ; C démarre pendant que B tourne : C **n'applique pas** la 0.1.1 (version installée toujours 0.1.0) ;
2. D, lancé pendant que B tourne, demande l'application : refusée (« another helper is running: close it, then restart to update »), version toujours 0.1.0 ;
3. tous arrêtés, E démarre seul : la 0.1.1 est appliquée au démarrage et E relancé ;
4. une 0.1.2 publiée dans le flux ; F, seul, la télécharge (différentiel) et l'applique sur demande : relancé en 0.1.2 ;
5. désinstallation : dossier et raccourcis retirés.

Puis, après la revue Copilot (verrou d'aiguillage `helper.gate`, application refusée pendant un téléchargement), les étapes 1 et 2 refaites avec le même résultat, et deux helpers lancés au même instant avec une mise à jour en attente : le premier applique et quitte, le second attend la fin de sa décision puis, le premier parti, se trouve seul et demande lui aussi l'application ; Velopack arrête le second `Update.exe` et un seul helper redémarre, en 0.1.1. Aucun helper qui jouait déjà n'est coupé.

Enfin, le 14 octobre 2026, avec le **vrai helper câblé** (`main.rs` de cette branche, après la fusion de l'icône de la zone de notification #12), build `--release`, empaqueté en 0.1.0, 0.1.1 et 0.1.2 (différentiels de 2 Ko, binaire identique), flux servi sur `127.0.0.1:8766`, helpers lancés sur des ports à part (47811 à 47813) :

1. `faulk-helper.exe --veloapp-install 0.0.1` (la sonde de la CI) : code 0 ; l'installeur silencieux exécute le crochet `--veloapp-install` en 148 ms ;
2. A (0.1.0, port 47811) lit le flux stable seulement (`?localVersion=0.1.0&id=FaulkHelper&stagingId=…`), télécharge le différentiel 0.1.1, journal `update: 0.1.1 downloaded, applied at the next start` dans `helper.log` ;
3. B démarre pendant que A tourne (port 47812) : il ne l'applique pas, version installée toujours 0.1.0 ;
4. A et B arrêtés, C démarre seul : la 0.1.1 est appliquée au démarrage, le helper relancé avec `--port 47811`, journal `FaulkHelper 0.1.1` ;
5. la 0.1.2 ajoutée au flux ; l'application **sur demande** n'ayant pas encore d'entrée de menu, elle est éprouvée avec le petit exécutable de test (mêmes appels, posé dans le dossier `current` de l'installation) : demandée pendant que le vrai helper C tourne, refusée (« another helper is running: close it, then restart to update ») ; C arrêté, demandée de nouveau : appliquée, et c'est le vrai helper qui redémarre, en 0.1.2, avec les mêmes arguments ;
6. désinstallation : dossier et raccourcis retirés ; puis une installation neuve avec une valeur « Lancer au démarrage de Windows » qui lance le helper installé : la désinstallation la retire.

## Ce que le fondateur doit faire

1. **Le jeton** : créer un jeton GitHub à grain fin, accès au seul dépôt `sprkl/faulk-release`, permission `Contents: Read and write`, puis l'enregistrer comme secret `RELEASES_TOKEN` de `sprkl/faulk` (`gh secret set RELEASES_TOKEN -R sprkl/faulk`). Noter sa date d'expiration : à son échéance, la copie publique échoue.
2. **Vercel** : dans le projet qui déploie `web/` (répertoire racine `web`), ajouter le domaine `get.faulk.app` (Settings → Domains), puis chez le registraire de `faulk.app` l'enregistrement DNS que Vercel affiche (en général un `CNAME get` vers `cname.vercel-dns.com`). Si `get.faulk.app` est rattaché à un autre projet, y copier les règles de `web/vercel.json`.
3. **Vérifier après déploiement** :
   ```bash
   curl -sI https://get.faulk.app/helper/stable/releases.win.json   # 307 vers .../releases/latest/download/releases.win.json
   curl -sI https://get.faulk.app/helper/packages/FaulkHelper-0.1.0-full.nupkg   # 307 vers .../download/helper-v0.1.0/...
   curl -sI https://get.faulk.app/helper/windows   # 307 vers ...-win-Setup.exe
   ```
4. **Première version** : une fois cette branche arrivée sur `main`, pousser `helper-v0.1.0` sur un commit de `main` (ou lancer le workflow depuis `main`).
5. **Plus tard** : le compte Azure Artifact Signing (Windows), le compte Apple Developer (macOS).

## Références

- [Velopack : démarrage en Rust](https://docs.velopack.io/getting-started/rust), [canaux](https://docs.velopack.io/packaging/channels), [hébergement](https://docs.velopack.io/distributing/overview), [signature](https://docs.velopack.io/packaging/signing)
- [Vercel : redirections de `vercel.json`](https://vercel.com/docs/project-configuration#redirects)
- [axoupdater](https://docs.rs/axoupdater), [self_update](https://docs.rs/self_update)
- [Azure Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)
