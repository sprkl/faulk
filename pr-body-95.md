## Pourquoi

SV-2 (#95) : un projet vit dans un vrai dossier (Chrome, Edge), s'enregistre tout seul, et se retrouve après un rechargement. L'observation du product owner est couverte : tant que des modifications ne sont pas enregistrées, le navigateur avertit avant de quitter ou recharger, et l'état « non enregistré / enregistré à hh:mm » est visible dans la barre du haut.

## Ce qui change

- **Commandes** : `project.new` (destructive, non annulable), `project.save` et `project.save_as` (`live`, sans effet sur le projet : la page écrit). Schéma, description, exemples, refus, parité MCP, `pnpm gen:protocol`. Les fichiers provisoires de PR-4 (`.faulk/interim-project.json`, `ProjectChanges`, `SaveReport`) sont supprimés.
- **Cœur** (`crates/project-wasm/src/save.rs`) : `pw_save(mode)` exécute `project::file::save` et `save_session` contre un enregistreur et rend les écritures en un cadre binaire (blobs, fichiers de chaque unité, `project.json` en dernier, suppressions, session). Le résultat reste en attente jusqu'à `pw_saved` : une écriture qui échoue ou un onglet fermé ne coûte rien, les mêmes écritures sont recalculées. Modes : `changed`, `forced`, `everything`, `copy` (nouvelle identité de projet, comme le dit le modèle), `rewrite` (garder « ma » version après une modification externe). Un changement de session (solo, sélection, armement, entrée MIDI) est un changement à enregistrer. Un projet ouvert garde l'état d'enregistrement de son dossier : un `.gitignore` ou `.gitattributes` de l'utilisateur n'est jamais réécrit.
- **Magasin** (`web/src/lib/storage/`) : `ProjectStore` dans un Worker (lire le dossier, lire une plage, écrire atomiquement, scanner), implémenté sur `FileSystemDirectoryHandle`. Remplacement d'un fichier : `createWritable()` seul (fichier d'échange remplacé à `close()`). Fichier neuf : écrit dans `.faulk/tmp/` puis `move()`, pour qu'un nom n'existe jamais vide (sans cela, un onglet fermé en pleine écriture laissait un `track.json` vide et le projet ne s'ouvrait plus : le test d'acceptation l'a montré) ; repli en place là où `move()` manque.
- **Contrôleur** (`storage.svelte.ts`) : enregistrement automatique (1,5 s après la dernière modification, 10 s au plus en continu, allongé selon le coût mesuré), écriture à la mise en arrière-plan, projets récents (IndexedDB), permission (`queryPermission` / `requestPermission`) avec bandeau, dossier refusé, relecture après modification externe (`FileSystemObserver`, repli au retour du focus), un onglet par projet (Web Locks), dernier projet rouvert au chargement.
- **Interface** : menu « Projet » dans la barre du haut (Nouveau, Ouvrir un dossier, Enregistrer, Enregistrer sous, récents), état visible, Ctrl/Cmd+S et Ctrl/Cmd+Maj+S (`keyboard.ts`), bandeaux et confirmation, textes EN et FR (tutoiement).

## Décisions prises (déléguées au lead)

1. **État visible** : sous le nom du projet, en toutes lettres : « Non enregistré » (pas de dossier), « Modifications non enregistrées », « Enregistrement… », « Enregistré à 14:32 », « Enregistrement en pause » (permission), « Échec de l'enregistrement », « Dossier modifié ». Sous 1400 px la barre est pleine : le projet devient une icône avec un point d'état, et le texte est dans l'infobulle, pour les lecteurs d'écran et en tête du menu.
2. **Un seul menu « Projet »** dans la barre du haut, plutôt qu'une vue « projets » (`views/projects/`) : suffisant pour le MVP.
3. **Enregistrement automatique** : 1,5 s après la dernière modification, 10 s au plus en continu ; le délai s'allonge avec le coût mesuré du calcul (au plus 3 % du fil principal, 60 s au plus) ; écrit aussi quand l'onglet passe en arrière-plan. Écarté : écrire à chaque modification (un glissement de potard en ferait des centaines).
4. **`beforeunload`** tant que des changements ne sont dans aucun dossier (y compris un projet sans dossier). Aucun avertissement pour un projet intact.
5. **Nouveau / Ouvrir sur un projet modifié** : avec un dossier, les changements sont écrits d'abord et l'utilisateur n'est interrogé que si l'écriture échoue ; sans dossier, confirmation (le bouton « Annuler » a le focus). Écarté : toujours demander.
6. **Navigateur sans File System Access** (Firefox, Safari) : Ouvrir et Enregistrer sous sont désactivés, le menu dit ce qui marche (le projet reste dans l'onglet, avertissement avant de partir) et recommande Chrome ou Edge ; Ctrl+S affiche la même explication. Le repli OPFS est SV-3 (#96).
7. **Enregistrer sous** n'écrit que dans un dossier **vide** (Faulk n'écrit jamais au milieu d'autres fichiers) ; depuis un projet qui a un dossier : nouvelle identité de projet, blobs copiés, ancien dossier intact.
8. **Au chargement** : le dernier projet rouvre seul si le navigateur garde la permission, sinon bandeau « Rouvrir … ? ».
9. **Modification externe** : sans changement non enregistré, rechargement silencieux avec un message (l'historique d'annulation est perdu) ; avec des changements, bandeau « Garder la mienne / Charger celle du dossier ». Les changements ne s'écrivent pas tant que l'utilisateur n'a pas choisi.
10. **Nouveau projet** : le projet par défaut (un synthé), nouvelle identité ; l'entrée MIDI reste choisie (c'est le matériel du musicien, pas le morceau). Le projet de départ de la page a désormais sa propre identité (il partageait une constante).
11. **Écartés du périmètre** (spec de stockage local) : le journal OPFS et la pause du transport pendant une perte de permission (les changements restent en mémoire, le bandeau et l'avertissement de fermeture restent) ; l'écriture du cache `.faulk/cache/` (ses estampilles viennent du dossier après l'écriture).

## Vérification

- Rust : `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` verts ; `project-wasm` (18+ tests de l'écriture), `commands/tests/project_store.rs`, exemples du contrat et parité MCP.
- Web : `pnpm gen:protocol` (régénéré après la fusion de `develop`), `pnpm typecheck`, `pnpm lint`, `pnpm test` (493 tests, dont 46 du contrôleur et du magasin).
- Playwright (Chromium, `showDirectoryPicker` simulé rendant un dossier OPFS) : `project-store.spec.ts` couvre modifier / recharger / retrouver, onglet fermé pendant une écriture (ancien fichier intact, projet rouvert), permission perdue (bandeau, geste, reprise) et rendue par le navigateur au rechargement (bandeau « Rouvrir »), dossier refusé / annulé / non vide, Nouveau avec confirmation, modification externe, deux onglets, navigateur sans dossier, FR, avertissement de fermeture. Aucune erreur console. Suite e2e complète sur les projets `dev` et `preview`.
- **Coût de l'enregistrement** (mesures de cette machine, build de dev) : projet moyen de 25 600 notes (16 pistes, 128 clips) : calcul sur le fil principal **2,6 à 3,3 ms** par enregistrement automatique d'une note modifiée, écriture dans le Worker 2 à 3,5 ms, 1 fichier ; premier « Enregistrer sous » de 310 fichiers : 8 ms de calcul, 750 ms d'écriture dans le Worker ; aucune tâche longue sur le fil principal, plus grand intervalle entre deux rapports du moteur pendant la lecture : 70 ms avec enregistrements contre 78 ms sans (le bruit de base). Profil QA-7 (200 000 notes, 6 millions de points d'automation), natif release : enregistrement après une modification 0,80 s, « Enregistrer sous » 0,84 s (156 Mio), enregistrement forcé sans rien à écrire 0,60 s : c'est pourquoi le délai s'allonge ; marquer les unités sales avec les événements des commandes le ramènerait à quelques ms (idée).
- **Non vérifié** : le comportement de `createWritable()` et de `move()` dans un dossier réellement choisi par l'utilisateur (le sélecteur natif ne se pilote pas sans interface). L'OPFS est le même type de poignée, mais Chrome peut y ajouter son contrôle Safe Browsing à la fermeture. À essayer à la main sous Chrome et Edge : choisir un dossier vide, modifier, recharger, fermer l'onglet pendant une modification. Pas non plus de latence audio mesurée (le chemin audio n'est pas touché).

## Judge Dredd

Voir le commentaire de l'issue et les sections ci-dessous.

## Hors périmètre

Voir les idées en fin de description.

Closes #95

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01SJ5mpZMZ2tFzSpRJZijt6Y
