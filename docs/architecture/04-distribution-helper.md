---
type: Architecture
title: "Distribution et mises à jour du helper"
description: "How the native helper is built, packaged with Velopack, published to a public Vercel Blob store, served through get.faulk.app redirects on Vercel, and updated on the user's machine (stable and beta channels, deltas, tray entries, signing to come)."
status: draft
---

# Distribution et mises à jour du helper

Ce document décrit la chaîne qui va du dépôt au helper installé sur un poste, puis la manière dont ce helper se met à jour. Le code reste privé, les binaires sont publics dans un magasin Vercel Blob, et le helper ne lit que notre domaine, qui redirige vers ce magasin.

> **État (14 octobre 2026).** Le module `crates/helper/src/updater.rs`, la CI `.github/workflows/release-helper.yml`, le script de publication `scripts/helper-release/` et les redirections `web/vercel.json` sont écrits et testés, et `main.rs` appelle l'updater (voir [Le câblage dans `main.rs`](#le-câblage-dans-mainrs)). La chaîne complète (installation de la 0.1.0, mise à jour vers la 0.1.1 en différentiel, passage en bêta, application au démarrage et sur demande, désinstallation) a été éprouvée sur un poste Windows avec un flux local, puis refaite avec le vrai helper câblé (voir [Éprouver la chaîne en local](#éprouver-la-chaîne-en-local)).
>
> Depuis #133 (décision du fondateur du 14 octobre 2026), les téléchargements et le flux viennent de Vercel Blob et non plus des releases GitHub. Reste à faire, dans cet ordre : publier la 0.1.0 dans le magasin (voir [Ce que le lead doit faire](#ce-que-le-lead-doit-faire)), éprouver un envoi réel par une pré-version, puis la signature.

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

## La distribution : un magasin Vercel Blob public

**Décidé par le fondateur le 14 octobre 2026** : les téléchargements et le flux de mise à jour sont servis par le magasin Vercel Blob `faulk-release` (public, région `cdg1`, rattaché au projet Vercel `faulk` qui déploie `web/`). Son adresse publique, lue avec `vercel blob get-store store_gpOtfUhRPU6RafCn --scope sparkletech` : `https://gpotfuhrpu6rafcn.public.blob.vercel-storage.com`.

Le montage est asymétrique :

- **en publication**, la CI envoie les fichiers produits par `vpk` dans le magasin avec le SDK officiel `@vercel/blob` (script `scripts/helper-release/publish.mjs`, version épinglée), avec le jeton lecture-écriture du secret GitHub `HELPER_RELEASES_READ_WRITE_TOKEN`, passé en `BLOB_READ_WRITE_TOKEN` ;
- **en consommation**, le helper ne lit jamais l'adresse du magasin : il lit `https://get.faulk.app/helper/...`, que Vercel redirige vers le magasin. L'URL inscrite dans le helper est la nôtre : l'hébergement a déjà changé une fois (des releases GitHub vers Blob) sans toucher aux helpers installés.

### Les chemins dans le magasin

Sans suffixe aléatoire, pour que chaque fichier ait une adresse stable :

| Chemin | Contenu | Écrasable | Cache (`cacheControlMaxAge`) |
|---|---|---|---|
| `helper/<os>/packages/FaulkHelper-<version>-full.nupkg`, `-delta.nupkg` | paquets de toutes les versions | **non** | 1 an |
| `helper/<os>/<canal>/FaulkHelper-<os>-Setup.exe`, `-Portable.zip` | installeur et version portable de la dernière version du canal | oui | 60 s |
| `helper/<os>/<canal>/assets.<os>.json`, `RELEASES` | index produits par `vpk` | oui | 60 s |
| `helper/<os>/<canal>/releases.<os>.json` | le flux du canal | oui, **écrit en dernier** | 60 s |

`<os>` est le canal Velopack du système (`win` seul publié aujourd'hui), `<canal>` vaut `stable`, ou `beta` pour une version à suffixe (`0.3.0-beta.1`). 60 s est le minimum de Vercel Blob, et le délai maximal pour qu'un fichier écrasé soit servi à jour.

Les règles de la publication (`scripts/helper-release/publish-core.mjs`, testées sans réseau ni jeton par `publish-core.test.mjs`) :

1. **les paquets d'abord**, jamais remplacés, et seulement ceux de la version publiée que cite son flux (un paquet de trop dans le dossier fait échouer la publication : il deviendrait un fichier immuable impossible à corriger) : un paquet absent est envoyé ; présent avec le même contenu (SHA-256), il est sauté ; présent avec un autre contenu, la publication échoue (« a published version is never replaced »). Un helper qui a déjà lu un flux ne trouve donc jamais un autre paquet que celui que le flux annonce ;
2. **chaque paquet que cite le nouveau flux doit être dans le magasin** (le flux cite aussi le paquet complet de la version précédente, base du différentiel) ; sinon le flux n'est pas écrit ;
3. **le magasin doit être celui vers lequel `get.faulk.app` redirige** (l'origine lue dans `web/vercel.json`) : un jeton d'un autre magasin est refusé avant toute écriture (l'identifiant du magasin, dans le jeton `vercel_blob_rw_<id>_…`, doit être l'hôte `<id en minuscules>.public.blob.vercel-storage.com`), et l'origine de chaque réponse est vérifiée ensuite ;
4. puis l'installeur et la version portable, les index, et **le flux en dernier**. Un client ne voit jamais un flux qui cite un paquet absent ; pendant les 60 s du cache, il peut encore voir l'ancien flux, dont les paquets existent toujours ;
5. **un canal ne recule jamais** : si le flux du canal annonce déjà une version plus récente, seuls les paquets sont envoyés ; la même version est republiée (relance). Deux publications d'un même canal ne tournent jamais ensemble (groupe de concurrence `helper-blob-<canal>` de la CI), et le flux est relu à une adresse propre à sa version (son etag, donné par l'API Blob), pas dans une copie en cache. GitHub ne garde qu'un job en attente par groupe : un troisième run annule celui qui attend, qui apparaît alors annulé ; il suffit de relancer ce job, la publication reprend là où elle en est ;
6. **Windows seulement** : le helper demande ses paquets à `get.faulk.app/helper/packages/<fichier>`, sans le système, et `web/vercel.json` mène cette adresse aux paquets Windows. La publication refuse donc tout autre système (`PUBLISHED_OS`) tant que le helper n'a pas un chemin de paquets par système.

`vpk pack` construit son flux à partir des seuls paquets présents dans le dossier de sortie (vérifié avec vpk 1.2.161 : un flux laissé là est ignoré) : le nouveau flux cite le paquet complet et le différentiel de la version, et le paquet complet de la précédente, téléchargé pour le différentiel.

`publish.mjs publish … --dry-run` fait tout cela contre un magasin en mémoire et affiche ce qu'il écrirait : chaque build d'essai de la CI le lance sur la vraie sortie de `vpk`.

### Les releases GitHub

**Décidé par le lead (#133)** : la release de `sprkl/faulk` (privée) est gardée, comme archive des fichiers et journal des changements (notes générées) ; elle ne coûte rien (`GITHUB_TOKEN`). La copie publique dans `sprkl/faulk-release`, la pré-version tournante `helper-beta` et le secret `RELEASES_TOKEN` sont abandonnés : ils n'existaient que pour rendre les fichiers publics, ce que fait Blob, et deux sources publiques pourraient diverger. Les releases `helper-v0.1.0` déjà publiées restent telles quelles.

## Les redirections de `get.faulk.app`

Elles vivent dans `web/vercel.json`, lu par le projet Vercel qui déploie `web/`. Chaque règle porte la condition `has: host = get.faulk.app` : `faulk.app` lui-même n'est pas touché. Toutes sont temporaires (`permanent: false`, code 307), pour pouvoir changer d'hébergement. Ce sont des redirections du proxy de Vercel, appliquées avant les fichiers produits par SvelteKit (`adapter-static`) : le cadre de l'application n'y change rien.

| Demandé à `get.faulk.app` | Redirigé vers (`https://gpotfuhrpu6rafcn.public.blob.vercel-storage.com/...`) | Pourquoi |
|---|---|---|
| `/helper/<canal>/releases.<os>.json` (`stable` ou `beta` ; `win`, `linux`, `osx`) | `helper/<os>/<canal>/releases.<os>.json` | le flux du canal ; une bêta n'est jamais publiée dans le dossier `stable`. |
| `/helper/packages/FaulkHelper-<version>-<full\|delta>.nupkg` | `helper/win/packages/<fichier>` | tous les paquets au même endroit : le flux d'une version cite aussi le paquet complet de la précédente. L'URL du client ne porte pas le système : macOS et Linux demanderont un chemin de paquets par système côté helper. |
| `/helper/windows` | `helper/win/stable/FaulkHelper-win-Setup.exe` | le lien de téléchargement à donner (le message de la page pour un helper trop ancien le cite). |
| tout le reste, racine comprise | le même installeur | **décision du lead** : `get.faulk.app` sert à télécharger le helper. |

Deux tests lisent `web/vercel.json`, seul endroit où figure l'adresse du magasin :

- `crates/helper/src/updater/tests.rs` vérifie que chaque URL construite par le helper aboutit au bon chemin du magasin, que toutes les règles mènent à une seule origine `https://<id>.public.blob.vercel-storage.com`, et que l'adresse de téléchargement citée par les textes de la page (`web/src/lib/i18n/app.ts`) mène à l'installeur par une règle à elle, sans compter sur la règle « tout le reste » ;
- `scripts/helper-release/publish-core.test.mjs` vérifie que ces mêmes URL aboutissent aux chemins où le script écrit.

Leur limite : ils traduisent eux-mêmes la syntaxe des sources (sous-ensemble de path-to-regexp). Le compilateur de routes de Vercel (`@vercel/routing-utils`, `getTransformedRoutes`) a accepté le fichier et donné les mêmes destinations le 14 octobre 2026 ; il ne remplace pas la vérification après déploiement (voir [Ce que le lead doit faire](#ce-que-le-lead-doit-faire)).

## Les canaux

- **Stable** : le helper lit le flux stable seulement, et en retire toute pré-version qui s'y trouverait par erreur. Deux barrières, donc : `releases/latest` côté GitHub, le filtre côté client.
- **Bêta** : le helper lit les deux flux et prend la version la plus récente. Un utilisateur bêta reçoit donc aussi une version stable plus récente que sa bêta. Si le flux bêta n'existe pas encore (aucune bêta publiée), le flux stable suffit.
- **Choix du canal** : celui de la version installée (une `0.3.0-beta.1` est en bêta). La variable d'environnement `FAULK_UPDATE_CHANNEL=stable|beta` le force : c'est un outil d'essai (comme `FAULK_UPDATE_URL`), pas un réglage proposé à l'utilisateur ; qui peut écrire l'environnement de l'utilisateur peut déjà exécuter du code sous son compte. Conséquence à connaître : un utilisateur bêta qui reçoit une version stable repasse en stable. **Décidé par le fondateur le 14 octobre 2026** : le canal suit la version installée, sans réglage ni mémoire du canal (une installation stable reçoit les versions stables, une installation de pré-version reçoit les pré-versions).
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
- **Vie privée** : chaque lecture du flux porte `?localVersion=<version>&id=FaulkHelper&stagingId=<identifiant>`. Ce paramètre est ajouté par Velopack ; `stagingId` est un identifiant aléatoire propre à l'installation, prévu pour les déploiements progressifs. Il arrive à Vercel et, par la redirection, au magasin Vercel Blob (la requête redirigée garde ses paramètres).
- **Tests de pointage** : la variable `FAULK_UPDATE_URL` remplace `https://get.faulk.app/helper` par une adresse `https://` (n'importe quel hôte) ou `http://` vers la boucle locale seulement (`127.0.0.1`, `localhost`, `[::1]`), sans requête ni fragment (les chemins du flux et des paquets y sont ajoutés). Les paquets ne sont vérifiés que contre le flux servi par cette même adresse : qui peut écrire l'environnement de l'utilisateur choisit le code installé. C'est accepté, car qui peut écrire cet environnement peut déjà exécuter du code sous ce compte ; la signature de code (plus bas) fermera ce point.

### Le câblage dans `main.rs`

`main.rs` appelle l'updater depuis le 14 octobre 2026 :

- `helper::updater::startup()` est la première instruction de `main`, avant la lecture des arguments, la prise du port (un helper par port, #12) et l'ouverture du journal. Un second helper lancé sur un port déjà servi passe donc d'abord par `startup()` : il voit l'autre helper au verrou d'instance et n'applique rien, puis ouvre la page et quitte.
- `Updater::start(Config::from_env())` est appelé une fois le serveur lancé ; l'updater vit aussi longtemps que le helper et n'est pas attendu à la sortie (`Quitter`) : une vérification ou un téléchargement en cours est abandonné avec le processus. Hors installation, le journal dit `update: disabled (...)`.
- **Désinstallation** : `startup()` inscrit un crochet Velopack `before_uninstall` qui retire la valeur « Lancer au démarrage de Windows » (clé `Run`, valeur `FaulkHelper`) quand elle lance l'exécutable désinstallé ; une valeur qui lance une autre copie (un build de développement) n'est pas touchée. Le crochet ouvre lui-même le journal pour y écrire ce qu'il a fait (`uninstall: start with Windows removed`). Sans cela, la désinstallation laissait une entrée de démarrage vers un fichier supprimé.

**Les entrées du menu de la zone de notification** (#133, textes décidés par le lead) :

- « Check for updates » / « Rechercher des mises à jour » : `updater.check_now()` ; grisée quand l'updater est désactivé (helper lancé par `cargo`) ;
- « Restart to update to {version} » / « Redémarrer pour passer à la {version} » : `updater.apply_and_restart()`. Elle apparaît quand l'updater passe `Ready`, change de texte si une version plus récente est téléchargée, reste pendant une vérification ou un téléchargement (le paquet déjà téléchargé reste applicable, et la cacher à chaque vérification périodique la ferait clignoter), et disparaît quand l'updater revient à un état sans paquet (`UpToDate`, `Failed`, `Idle`, `Disabled`). La règle est une fonction pure testée pour chaque état (`RestartEntry::follow_status`, `tray_text.rs`), et son application au vrai menu (insertion juste après « Rechercher des mises à jour », nouveau texte, grisé, retrait) est testée sous Windows (`UpdateItems`, `tray.rs`). Le clic sur un helper installé n'a pas encore été éprouvé. Pendant une vérification ou un téléchargement, elle est grisée (l'application serait refusée). Au clic, l'icône est retirée (Velopack quitte le processus en cas de succès, et Windows garderait une icône morte), puis réaffichée si l'application est refusée.

Limites connues : un refus (« another helper is running ») n'est écrit qu'au journal, et « Rechercher des mises à jour » ne montre pas de résultat tant qu'aucune mise à jour n'est prête ; un retour visible demande de nouveaux textes, à décider. L'application quitte le processus sans le relâchement des voix que fait « Quitter » : une note tenue est coupée net (`apply_and_restart` le dit déjà : la sortie audio s'arrête le temps de la relance).

Sans ces appels, un installeur produit par la CI installerait un helper qui ne répond pas aux arguments `--veloapp-*` et ne se met jamais à jour. `vpk` ne le détecte pas : la CI le vérifie elle-même et **refuse de publier** une version sans ces appels (un build d'essai passe, avec un avertissement). Elle lance le binaire construit comme le ferait l'installeur (`timeout 30 faulk-helper.exe --veloapp-install 0.0.1`) : un helper câblé répond aussitôt par 0 ; un helper non câblé refuse l'argument inconnu (code 1, et dans un build de release une boîte d'erreur qui le bloque jusqu'à l'expiration du délai). `Updater::start` ne s'observe pas hors installation : la CI cherche son appel, hors commentaire, dans `main.rs`.

Ces actions de la zone de notification concernent le helper lui-même, pas le projet musical. Elles ne passent pas par `crates/commands`. Si l'interface web doit un jour les proposer, la question du passage par une commande sera à trancher à ce moment-là.

## Publier une version

`.github/workflows/release-helper.yml`, en cinq jobs :

1. **version** : depuis le tag `helper-v<version>`, la saisie d'un lancement manuel, ou l'entrée `mirror`. Format vérifié (`X.Y.Z` ou `X.Y.Z-suffixe`), un suffixe fait une pré-version, publiée sur le canal `beta`. Un lancement manuel avec version ne publie que depuis `main` (stable) ou `main`/`develop` (pré-version). Même règle pour un tag poussé : un tag stable doit être sur un commit de `main`, un tag de pré-version sur `main` ou `develop`, sinon le run échoue avant de construire.
2. **windows** (obligatoire) : tests du helper, build `--release`, téléchargement depuis le magasin du dernier paquet complet stable **plus ancien que la version construite** (`publish.mjs previous`, empreinte vérifiée contre le flux ; pour le différentiel, sauf pré-version et build d'essai ; un correctif plus ancien que la dernière version publiée part ainsi de la version qui le précède), `vpk pack --runtime win-x64`, artefact de cette version seule, téléchargeable quatorze jours. Un build d'essai lance la publication à blanc sur cet artefact.
3. **unix** (macOS arm64 et Linux x64, facultatifs, `continue-on-error`) : build et empaquetage pour mémoire, **jamais publiés**, ni signés ni testés.
4. **release** (seulement avec une version) : refuse un helper non câblé, publie dans le magasin Blob (voir plus haut), puis crée la release dans `sprkl/faulk` (notes générées ; une release qui existe déjà voit ses fichiers remplacés). Blob d'abord : une publication refusée (paquets reconstruits) ne remplace pas non plus les fichiers de l'archive. Sans le secret `HELPER_RELEASES_READ_WRITE_TOKEN`, le job échoue. Un lancement en brouillon (`draft`) ne publie rien dans le magasin. Relancer **ce job seul** reprend les fichiers construits et termine une publication interrompue ; relancer tout le workflow reconstruit des paquets différents, que la publication refuse : il faut alors une nouvelle version.
5. **mirror** (seulement avec `mirror`) : publie dans le magasin les fichiers d'une release existante de `sprkl/faulk`, sans construire. Sert à la 0.1.0, publiée avant Blob.

Une version ne devient `latest` que si elle est plus récente que l'actuelle : relancer un ancien tag ne fait pas reculer le canal stable.

Quatre façons de lancer :

- la voie normale : `git tag helper-v0.2.0 && git push origin helper-v0.2.0` ;
- sans tag : `gh workflow run release-helper.yml --ref main -f version=0.2.0` ;
- une release d'avant Blob : `gh workflow run release-helper.yml --ref develop -f mirror=helper-v0.1.0` ;
- un build d'essai : `gh workflow run release-helper.yml` sans version, ou toute pull request qui modifie ce workflow, l'updater, `main.rs`, `lib.rs`, `crates/helper/Cargo.toml`, `Cargo.lock`, `scripts/helper-release/` ou `web/vercel.json` (le câblage, la version de Velopack et la publication passent ainsi la garde dès la PR).

`gh workflow run` ne fonctionne qu'une fois le workflow présent sur la branche par défaut (`develop`).

## Compatibilité avec la page

Le helper se met désormais à jour à son rythme, indépendamment du déploiement de `faulk.app`.

**Règle, décidée par le fondateur le 14 octobre 2026** : le helper et la page doivent avoir exactement la même `PROTOCOL_VERSION` (`crates/protocol`). La page refuse déjà tout helper d'une autre version (`web/src/lib/engine/helper-client.ts`, état `incompatible` de la session). Pas de plage de versions acceptées.

Conséquence : **un changement de protocole se livre d'un bloc**, la version du helper publiée et la page déployée ensemble. Tant que l'un des deux manque, le mode Boost est coupé pour tous les utilisateurs dont le helper et la page ne sont plus d'accord, et la page reste en mode Web avec un message selon le sens (#133, textes décidés par le lead) :

- helper plus ancien que la page (`helper.helperOlder`) : « This helper is older than the page (protocol {v}). Restart it to install its update, or download it again from get.faulk.app/helper/windows. » / « Ce helper est plus ancien que la page (protocole {v}). Relance-le pour installer sa mise à jour, ou retélécharge-le sur get.faulk.app/helper/windows. » ;
- helper plus récent (`helper.helperNewer`, page pas encore déployée ou ancienne page en cache) : « This page is older than your helper (protocol {v}). Reload the page. » / « Cette page est plus ancienne que ton helper (protocole {v}). Recharge la page. »

Le choix est fait par `incompatibleMessage` (`web/src/lib/engine/helper-client.ts`) ; l'adresse citée est une route réelle, vérifiée par un test (voir plus haut).

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

Le job `unix` construit et empaquette sans publier. Avant de publier : choisir les architectures (arm64 et x64 sur macOS), signer et notariser macOS, éprouver installation et mise à jour sur un poste de chaque système, publier leur sortie avec `publish.mjs --os osx|linux` (les redirections des flux les acceptent déjà), donner au helper un chemin de paquets par système (les noms de paquets de Velopack ne portent pas le système, et `/helper/packages/` mène aux paquets Windows), et un lien de téléchargement par système (`/helper/macos`, `/helper/linux`).

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

## Ce que le lead doit faire

Le secret `HELPER_RELEASES_READ_WRITE_TOKEN` et le magasin existent (fondateur, 14 octobre 2026). Le projet Vercel `faulk` déploie la production depuis `main`, et `get.faulk.app` y est rattaché et vérifié (lu le 14 octobre 2026 avec `vercel api /v9/projects/faulk` et `/v9/projects/faulk/domains`).

1. **Ordre de la bascule** : tant que le magasin est vide, `get.faulk.app` renverrait des 404 (installeur et flux). La production venant de `main`, la fusion dans `develop` ne change pas encore `get.faulk.app`. Une fois cette branche sur `develop`, et **avant** la release de `develop` vers `main`, publier la 0.1.0 dans le magasin :
   ```bash
   gh workflow run release-helper.yml --ref develop -f mirror=helper-v0.1.0
   ```
   Les helpers 0.1.0 déjà installés supportent une éventuelle fenêtre : un flux absent est réessayé à la vérification suivante.
2. **Vérifier après déploiement** :
   ```bash
   curl -sI https://get.faulk.app/helper/stable/releases.win.json   # 307 vers .../helper/win/stable/releases.win.json
   curl -sI https://get.faulk.app/helper/packages/FaulkHelper-0.1.0-full.nupkg   # 307 vers .../helper/win/packages/...
   curl -sI https://get.faulk.app/helper/windows   # 307 vers .../helper/win/stable/FaulkHelper-win-Setup.exe
   curl -sI https://gpotfuhrpu6rafcn.public.blob.vercel-storage.com/helper/win/stable/releases.win.json   # 200
   ```
3. **Éprouver un envoi réel** sans toucher au canal stable : une pré-version depuis `develop`, `gh workflow run release-helper.yml --ref develop -f version=0.1.1-beta.1` ; elle n'atteint que le dossier `beta`.
4. **Changer de magasin** : remplacer l'origine dans les quatre règles de `web/vercel.json` (les tests en dépendent, rien d'autre), et le secret. La publication refuse, avant toute écriture, un jeton d'un autre magasin que celui de `web/vercel.json`.
5. **Plus tard** : le compte Azure Artifact Signing (Windows), le compte Apple Developer (macOS).

## Références

- [Velopack : démarrage en Rust](https://docs.velopack.io/getting-started/rust), [canaux](https://docs.velopack.io/packaging/channels), [hébergement](https://docs.velopack.io/distributing/overview), [signature](https://docs.velopack.io/packaging/signing)
- [Vercel : redirections de `vercel.json`](https://vercel.com/docs/project-configuration#redirects), [Vercel Blob](https://vercel.com/docs/vercel-blob), [SDK `@vercel/blob`](https://vercel.com/docs/vercel-blob/using-blob-sdk)
- [axoupdater](https://docs.rs/axoupdater), [self_update](https://docs.rs/self_update)
- [Azure Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)
