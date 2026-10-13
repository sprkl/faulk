---
type: Architecture
title: "Distribution et mises à jour du helper"
description: "How the native helper is built, packaged with Velopack, published to the public sprkl/faulk-release repository, served through get.faulk.app redirects on Vercel, and updated on the user's machine (stable and beta channels, deltas, signing to come)."
status: draft
---

# Distribution et mises à jour du helper

Ce document décrit la chaîne qui va du dépôt au helper installé sur un poste, puis la manière dont ce helper se met à jour. Le modèle est celui de lebureau-desktop : le code reste privé, les binaires sont publics dans un dépôt de releases à part, et le helper ne lit que notre domaine, qui redirige vers ces releases.

> **État (14 octobre 2026).** Le module `crates/helper/src/updater.rs`, la CI `.github/workflows/release-helper.yml` et les redirections `web/vercel.json` sont écrits et testés. La chaîne complète (installation de la 0.1.0, mise à jour vers la 0.1.1 en différentiel, passage en bêta, application au démarrage et sur demande, désinstallation) a été éprouvée sur un poste Windows avec un flux local (voir [Éprouver la chaîne en local](#éprouver-la-chaîne-en-local)).
>
> Reste à faire, dans cet ordre : appeler l'updater depuis `main.rs` (en attente de l'icône de la zone de notification, #12), le secret `RELEASES_TOKEN`, le domaine `get.faulk.app` sur Vercel, une première version publiée par la CI, puis la signature.

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
- **Choix du canal** : celui de la version installée (une `0.3.0-beta.1` est en bêta). La variable d'environnement `FAULK_UPDATE_CHANNEL=stable|beta` le force. Conséquence à connaître : un utilisateur bêta qui reçoit une version stable repasse en stable. **Décision produit à confirmer** : garder ce comportement, ou mémoriser le canal (réglage, menu de la zone de notification).
- **Différentiels** : seulement sur le canal stable. Un différentiel ne s'applique qu'à la version à partir de laquelle il a été construit, et la CI le construit toujours à partir de la dernière version stable. Un poste bêta (bêtas successives, puis une stable construite sur une autre base) ne suit pas cette ligne : le canal bêta ignore les différentiels et télécharge les paquets complets.

## Côté helper : `crates/helper/src/updater.rs`

- `updater::startup()` : premier appel de `main`, avant la lecture des arguments. L'installeur et l'outil de mise à jour lancent l'exécutable avec des arguments `--veloapp-*` et attendent qu'il quitte aussitôt. Il applique aussi une mise à jour téléchargée lors d'une exécution précédente, puis relance le helper sur la nouvelle version, **sauf si un autre helper tourne déjà** : appliquer remplace les fichiers de l'installation et couperait le son de l'autre helper. Le premier helper garde pour cela un verrou (`<config>/faulk/helper.lock`) toute sa vie.
- `Updater::start(Config::from_env())` : lance un thread `faulk-updater`. Vérification au démarrage, puis toutes les six heures. Une version trouvée est téléchargée en arrière-plan. Hors installation (`cargo run`), l'updater est désactivé et ne touche pas le réseau.
- `Updater::check_now()` : vérification immédiate, sans attendre ; les demandes répétées pendant une vérification se fondent en une seule.
- `Updater::status()` : `Disabled`, `Idle`, `Checking`, `UpToDate`, `Downloading { percent }`, `Ready`, `Failed { error }`. Les erreurs sont classées : réseau, flux absent (404), flux invalide, paquet corrompu, autre. Un échec est retenté à la vérification suivante. Une version déjà téléchargée reste applicable après un échec.
- `Updater::apply_and_restart()` : applique la version téléchargée et relance le helper avec les mêmes arguments. Le processus quitte : la sortie audio s'arrête le temps de la relance, c'est voulu et c'est l'utilisateur qui le demande.
- **Audio** : l'updater ne s'exécute jamais sur le thread audio et ne partage aucun état avec lui. Ses seuls effets sur le son sont les deux sorties de processus ci-dessus.
- **Journal** : les lignes `update: ...` sur la sortie d'erreur, et les avertissements de Velopack, qui passent par un petit journal installé par `startup()`.
- **Vie privée** : chaque lecture du flux porte `?localVersion=<version>&id=FaulkHelper&stagingId=<identifiant>`. Ce paramètre est ajouté par Velopack ; `stagingId` est un identifiant aléatoire propre à l'installation, prévu pour les déploiements progressifs. Il arrive à Vercel et, par la redirection, à GitHub.
- **Tests de pointage** : la variable `FAULK_UPDATE_URL` remplace `https://get.faulk.app/helper`, uniquement par une adresse `https://` ou `http://` vers la boucle locale (`127.0.0.1`, `localhost`, `[::1]`).

### Le câblage dans `main.rs`

`main.rs` est en cours de modification pour l'icône de la zone de notification (#12) ; il n'appelle pas encore l'updater. Les lignes à ajouter :

```rust
// Tout en haut de main(), avant args() :
helper::updater::startup();
// Après le démarrage du serveur, gardé vivant aussi longtemps que le helper :
let updater = helper::updater::Updater::start(helper::updater::Config::from_env());
// Menu de la zone de notification :
//   « Rechercher des mises à jour » -> updater.check_now()
//   « Redémarrer pour mettre à jour » (si status() == Ready) -> updater.apply_and_restart()
```

Tant que ces lignes manquent, un installeur produit par la CI installerait un helper qui ne répond pas aux arguments `--veloapp-*` et ne se met jamais à jour. `vpk` ne le détecte pas : la CI le vérifie elle-même et **refuse de publier** une version sans ces appels (un build d'essai passe, avec un avertissement).

Ces actions de la zone de notification concernent le helper lui-même, pas le projet musical. Elles ne passent pas par `crates/commands`. Si l'interface web doit un jour les proposer, la question du passage par une commande sera à trancher à ce moment-là.

## Publier une version

`.github/workflows/release-helper.yml`, en quatre jobs :

1. **version** : depuis le tag `helper-v<version>` ou la saisie d'un lancement manuel. Format vérifié (`X.Y.Z` ou `X.Y.Z-suffixe`), un suffixe fait une pré-version. Un lancement manuel avec version ne publie que depuis `main` (stable) ou `main`/`develop` (pré-version).
2. **windows** (obligatoire) : tests du helper, build `--release`, téléchargement de la dernière release stable publique (pour le différentiel, sauf pré-version), `vpk pack --runtime win-x64`, artefacts téléchargeables quatorze jours.
3. **unix** (macOS arm64 et Linux x64, facultatifs, `continue-on-error`) : build et empaquetage pour mémoire, **jamais publiés**, ni signés ni testés.
4. **release** (seulement avec une version) : refuse un helper non câblé, crée la release dans `sprkl/faulk` (notes générées), puis la copie publique dans `sprkl/faulk-release` avec des notes fixes (les notes générées citent les pull requests privées). Sans `RELEASES_TOKEN`, la copie est sautée avec un avis explicite dans le résumé du run. Une pré-version met aussi à jour le flux de la release `helper-beta`, sauf en brouillon (ses paquets ne seraient pas téléchargeables).

Une version ne devient `latest` que si elle est plus récente que l'actuelle : relancer un ancien tag ne fait pas reculer le canal stable.

Trois façons de lancer :

- la voie normale : `git tag helper-v0.2.0 && git push origin helper-v0.2.0` ;
- sans tag : `gh workflow run release-helper.yml --ref main -f version=0.2.0` ;
- un build d'essai : `gh workflow run release-helper.yml` sans version, ou toute pull request qui modifie ce workflow ou l'updater.

`gh workflow run` ne fonctionne qu'une fois le workflow présent sur la branche par défaut (`develop`).

## Compatibilité avec la page

Le helper se met désormais à jour à son rythme, indépendamment du déploiement de `faulk.app`. Or la page refuse tout helper dont `PROTOCOL_VERSION` diffère du sien (`crates/protocol`, `web/src/lib/engine/`). Une version du helper qui change le protocole avant que la page soit déployée, ou l'inverse, coupe le mode Boost de tous les helpers à jour. Règle à suivre en attendant mieux : déployer la page et publier le helper ensemble quand le protocole change. Une règle de compatibilité (plage de versions acceptées) est à décider.

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

Ce qui a été fait le 14 octobre 2026 sur Windows 11, avec un `main.rs` temporairement câblé (non commité) :

1. `cargo build -p helper --release`, copie de `faulk-helper.exe` seul dans un dossier `app/` ;
2. `vpk pack --packId FaulkHelper --runtime win-x64 --packDir app --mainExe faulk-helper.exe --packVersion 0.1.0 --outputDir R`, copie de `FaulkHelper-win-Setup.exe`, puis la même commande en `0.1.1` (différentiel construit depuis la 0.1.0) et en `0.2.0-beta.1` dans un autre dossier ;
3. un dossier servi par `python -m http.server 8765 --bind 127.0.0.1` qui reproduit les redirections : `stable/releases.win.json`, `beta/releases.win.json`, `packages/*.nupkg` ;
4. installation silencieuse de la 0.1.0 (`Setup.exe --silent`), lancement avec `FAULK_UPDATE_URL=http://127.0.0.1:8765` : le helper lit le flux stable seulement (pas la bêta), télécharge le différentiel 0.1.1, passe `Ready` ;
5. second lancement : la 0.1.1 est appliquée au démarrage, le helper relancé avec les mêmes arguments ;
6. `FAULK_UPDATE_CHANNEL=beta` : la `0.2.0-beta.1` est téléchargée en paquet complet ; un second helper lancé pendant ce temps **n'applique pas** la mise à jour (le premier continue de jouer) ;
7. une `0.2.0-beta.2` ajoutée au flux bêta : appliquée sur demande (`apply_and_restart`), le helper relancé en `0.2.0-beta.2` ;
8. désinstallation : `%LocalAppData%\FaulkHelper\Update.exe --uninstall --silent` retire le dossier et les raccourcis.

## Ce que le fondateur doit faire

1. **Le jeton** : créer un jeton GitHub à grain fin, accès au seul dépôt `sprkl/faulk-release`, permission `Contents: Read and write`, puis l'enregistrer comme secret `RELEASES_TOKEN` de `sprkl/faulk` (`gh secret set RELEASES_TOKEN -R sprkl/faulk`). Noter sa date d'expiration : à son échéance, la copie publique échoue.
2. **Vercel** : dans le projet qui déploie `web/` (répertoire racine `web`), ajouter le domaine `get.faulk.app` (Settings → Domains), puis chez le registraire de `faulk.app` l'enregistrement DNS que Vercel affiche (en général un `CNAME get` vers `cname.vercel-dns.com`). Si `get.faulk.app` est rattaché à un autre projet, y copier les règles de `web/vercel.json`.
3. **Vérifier après déploiement** :
   ```bash
   curl -sI https://get.faulk.app/helper/stable/releases.win.json   # 307 vers .../releases/latest/download/releases.win.json
   curl -sI https://get.faulk.app/helper/packages/FaulkHelper-0.1.0-full.nupkg   # 307 vers .../download/helper-v0.1.0/...
   curl -sI https://get.faulk.app/helper/windows   # 307 vers ...-win-Setup.exe
   ```
4. **Première version** : une fois `main.rs` câblé, pousser `helper-v0.1.0` (ou lancer le workflow depuis `main`).
5. **Plus tard** : le compte Azure Artifact Signing (Windows), le compte Apple Developer (macOS).

## Références

- [Velopack : démarrage en Rust](https://docs.velopack.io/getting-started/rust), [canaux](https://docs.velopack.io/packaging/channels), [hébergement](https://docs.velopack.io/distributing/overview), [signature](https://docs.velopack.io/packaging/signing)
- [Vercel : redirections de `vercel.json`](https://vercel.com/docs/project-configuration#redirects)
- [axoupdater](https://docs.rs/axoupdater), [self_update](https://docs.rs/self_update)
- [Azure Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)
