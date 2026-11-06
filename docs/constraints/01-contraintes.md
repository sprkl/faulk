---
type: Constraints
title: "Contraintes transverses"
description: "Real-time audio, latency targets, browser limits, security of the local bridge, licences (VST3, ASIO, GPL), and the legal stance on recreating existing plugins."
status: draft
---

# Contraintes transverses

## Temps réel

Le chemin audio n'alloue pas, ne verrouille pas, ne bloque pas et ne panique pas. Règles détaillées dans `CLAUDE.md` à la racine du dépôt.

## Latence

| Mode | Cible | Limite connue |
| --- | --- | --- |
| Web | La plus basse possible, mesurée | Pas d'ASIO ni de mode exclusif dans le navigateur ; mixeur du système ; `renderSizeHint` (Chrome 153+) permet des blocs de 64 échantillons |
| Boost | Sous 10 ms en sortie | Dépend du pilote et de la carte son |

Une latence n'est jamais annoncée sans mesure.

## Navigateurs

- Web MIDI : Chrome, Edge, Firefox ; pas Safari.
- `SharedArrayBuffer` exige l'isolation cross-origin (en-têtes COOP et COEP, ou `Document-Isolation-Policy` dans Chromium). Décision du fondateur du 2026-10-14 : l'isolation est acceptée, parce que le streaming des pistes audio et le canal vers le moteur en ont besoin. Mise en place par la tâche AU-0 du [découpage du MVP](../specs/02-decoupage-mvp.md) : **tout le site** (page d'accueil comprise) est servi avec `Cross-Origin-Opener-Policy: same-origin` et `Cross-Origin-Embedder-Policy: require-corp`, déclarés une seule fois dans `web/vercel.json` (source `/(.*)`) et repris tels quels par `pnpm dev` et `pnpm preview` (`web/scripts/vercel-headers.ts`). Le code n'utilise pas encore la mémoire partagée (commandes par `postMessage`) : c'est l'affaire de CT-3 et MO-2. Conséquences :
  - **tout le site, pas seulement `/app`** : un Worker dédié (prévu par CT-3) ne se charge depuis une page isolée que si la réponse de son propre script porte COEP (vérifié le 2026-10-14 dans Chromium 156, Chrome 155 et Edge 155 : le Worker est refusé sans l'en-tête). Ses fichiers ont des noms à empreinte, dans `_app/` en production et sous `/src/` en dev : seul un en-tête sur tout le site les couvre sûrement. La page d'accueil ne charge que des ressources de sa propre origine et fonctionne isolée (polices, animation WebGL, tests `web/tests/e2e/`) ;
  - **aucune ressource tierce** sans CORS ni `Cross-Origin-Resource-Policy`, sur aucune page : polices et bibliothèques sont empaquetées par Vite ; une future intégration tierce (vidéo, formulaire, mesure d'audience hébergée ailleurs) devra être auto-hébergée, servie avec CORP, ou faire restreindre la règle de `vercel.json` ;
  - **fenêtres d'autres origines** : `COOP: same-origin` coupe le lien (`window.opener`) avec une fenêtre d'une autre origine, par exemple une fenêtre d'authentification OAuth ; à prévoir si Faulk ajoute un compte en ligne ;
  - **`require-corp` plutôt que `credentialless`** : `credentialless` n'existe pas dans Safari, et tout est de même origine ; `Document-Isolation-Policy` reste propre à Chromium et n'est pas utilisée ;
  - les WebSockets vers le helper (`ws://127.0.0.1:47800`) ne sont pas soumis à COEP : sonde, appairage et session fonctionnent sous isolation (tests `web/tests/e2e/helper.spec.ts`, et helper réel vérifié dans Chrome et Edge le 2026-10-14).
- Chrome ne transmet pas un `WebAssembly.Module` compilé à un `AudioWorklet` : on envoie les octets du module.
- Le helper sur `localhost` déclenche, depuis une page publique comme `https://www.faulk.app`, une demande d'autorisation d'accès au réseau local dans Chrome et Edge (Local Network Access, WebSockets compris depuis Chrome 147 ; constatée avec Chrome et Edge 155, voir [Distribution du helper](../architecture/04-distribution-helper.md#laccès-au-réseau-local-dans-chrome-et-edge)) : l'interface doit l'expliquer. Une page servie en local n'en déclenche pas.

## Sécurité du pont local

N'importe quel site ouvert dans le navigateur peut tenter de joindre un serveur sur `localhost`. Le helper :

- n'écoute que sur `127.0.0.1` ;
- exige un jeton, créé lors d'une confirmation de l'utilisateur, sur chaque connexion de la page (le jeton de session, donné à l'appairage, pour la session Boost comme pour le lien de contrôle des agents, #608) et sur chaque requête au serveur MCP en HTTP (un jeton distinct, créé seulement quand l'utilisateur lance `faulk-helper mcp-token` ; AG-1, #97) ;
- vérifie l'en-tête `Host` (contre le DNS rebinding) et l'en-tête `Origin` : seules les origines de Faulk ouvrent une session de page, et une requête MCP qui porte un `Origin` (donc venue d'une page web, celle de Faulk comprise) est refusée ;
- ne lance que les agents ACP que l'utilisateur a configurés, jamais une commande reçue de la page ;
- ne lit et n'écrit, en mode Boost, que dans le dossier du projet que la page a ouvert (décision du fondateur du 2026-11-06, AU-7, #116). Aucun message ne porte de chemin : l'utilisateur montre le dossier dans la fenêtre du helper, et le helper le reconnaît ensuite par l'identifiant du projet et par un marqueur que la page y écrit. Chaque chemin est contrôlé : comme texte d'abord (pas de `..`, de chemin absolu, de lecteur, d'UNC ni de nom de périphérique), puis sur le disque, où son chemin canonique doit rester dans le dossier (un lien symbolique ou une jonction qui en sort est refusé). Seule exception, sa propre configuration : la liste des dossiers qu'on lui a montrés ([stockage local](../architecture/03-stockage-local.md) § 5.13). Ce qu'il écrit dans ce dossier se limite à l'export audio, dans `exports/`, sans jamais remplacer un fichier (#319).

Les commandes destructrices demandent une confirmation de l'utilisateur, quel que soit l'appelant. Écart connu jusqu'à AG-4 (#100) : depuis AG-1 (#97), un agent MCP les exécute sans confirmation (elles restent annulables, sauf `project.new` qui garde d'abord le projet courant).

## Licences

- **VST3** : SDK sous licence MIT depuis la version 3.8 (octobre 2025).
- **ASIO** : SDK sous double licence, propriétaire ou GPLv3 (fin 2025). Utiliser la GPLv3 rend le helper GPLv3 : décision à prendre avant d'intégrer ASIO.
- **Plugins open source portés** : beaucoup sont GPL (Surge XT, Dexed) ; Airwindows est MIT. Un module dérivé de code GPL est GPL.

## Recréer des plugins existants

- **Interdit** : décompiler un plugin commercial pour en reproduire le code. Les licences l'interdisent et la reproduction serait une contrefaçon.
- **Autorisé** : la recréation en boîte noire par mesure (signaux de test, comparaison des réponses), qui reproduit un comportement sans copier de code. Ne jamais reprendre le nom, la marque ou l'apparence du plugin d'origine.
