# Modulo Allineatori (italiano).
# Chiavi del menu: aligners-menu-<percorso>; il percorso è l'id del comando `aligners.<percorso>` con i punti
# sostituiti da trattini (gli id Fluent non possono contenere punti). Descrizioni: aligners-desc-<percorso>.

aligners-name = Allineatori
aligners-select-hint = Seleziona una voce del menu Allineatori.
aligners-status-stub = Non ancora disponibile
aligners-status-stub-detail = Questa funzione è prevista e non è ancora implementata.
aligners-staging-stages = Stadi
aligners-staging-col-stage = Stadio
aligners-staging-col-teeth = Denti in movimento
aligners-staging-col-max-translation = Traslazione massima
aligners-staging-col-max-rotation = Rotazione massima
aligners-staging-col-ipr = Stripping
aligners-staging-no-stages = Nessuno stadio per ora.
aligners-staging-limits = Limiti per allineatore
aligners-staging-step-linear = Traslazione (mm)
aligners-staging-step-vertical = Intrusione / estrusione (mm)
aligners-staging-step-rotation = Rotazione (gradi)
aligners-staging-step-rotation-round = Rotazione, corone tonde (gradi)
aligners-staging-step-angular = Torque e tip (gradi)
aligners-staging-max-crown-step = Limite per un punto della corona (mm)
aligners-staging-interval-days = Giorni per allineatore

aligners-menu-case = Caso
aligners-desc-case = Importa le scansioni del paziente e imposta il caso.

aligners-menu-case-import-scans = Importa scansioni
aligners-desc-case-import-scans = Carica le scansioni delle arcate superiore e inferiore (STL, PLY, OBJ). L'importazione non avvia nessuna elaborazione.

aligners-menu-case-units-orientation = Unità e orientamento
aligners-desc-case-units-orientation = Verifica l'unità di misura delle scansioni (mm, cm, pollici) e l'orientamento del modello nel caso.

aligners-menu-case-case-info = Dati del caso
aligners-desc-case-case-info = Codice paziente pseudonimizzato e note del caso.

aligners-menu-preparation = Preparazione
aligners-desc-preparation = Porta le scansioni in un riferimento comune e individua i denti.

aligners-menu-preparation-orientation = Orientamento automatico e arcata
aligners-desc-preparation-orientation = Arcata superiore o inferiore, piano occlusale e linea mediana: il riferimento su cui lavorano tutte le fasi successive.

aligners-menu-preparation-orientation-automatic = Automatico
aligners-desc-preparation-orientation-automatic = Riconosce automaticamente l'arcata e il suo orientamento.

aligners-menu-preparation-orientation-manual = Manuale: scegli l'arcata
aligners-desc-preparation-orientation-manual = Scegli l'arcata e imposta a mano il piano occlusale e la linea mediana.

aligners-menu-preparation-numbering = Numerazione dei denti (FDI)
aligners-desc-preparation-numbering = Un punto (seme) sulla faccia occlusale di ogni dente con il suo numero FDI: indica al programma quali denti ci sono e dove.

aligners-menu-preparation-numbering-automatic = Trova i denti in automatico
aligners-desc-preparation-numbering-automatic = Individua i denti e li numera automaticamente.

aligners-menu-preparation-numbering-place-seeds = Manuale: posiziona i semi
aligners-desc-preparation-numbering-place-seeds = Posiziona un seme su ogni dente con il mouse e scegli il suo numero FDI.

aligners-menu-preparation-numbering-renumber = Manuale: cambia il numero di un dente
aligners-desc-preparation-numbering-renumber = Cambia il numero FDI di un dente o scambia due denti.

aligners-menu-preparation-numbering-mark-missing = Manuale: segna un dente assente
aligners-desc-preparation-numbering-mark-missing = Segna un dente come assente: il suo spazio resta vuoto.

aligners-menu-preparation-segmentation = Segmentazione
aligners-desc-preparation-segmentation = Ogni punto della scansione viene assegnato a un dente o alla gengiva. Il confine tra i colori diventa il margine della corona.

aligners-menu-preparation-segmentation-automatic = Automatica
aligners-desc-preparation-segmentation-automatic = Separa automaticamente i denti dalla gengiva.

aligners-menu-preparation-segmentation-manual-brush = Pennello manuale
aligners-desc-preparation-segmentation-manual-brush = Dipingi a mano le etichette dei denti con un pennello, alla massima risoluzione.

aligners-menu-preparation-segmentation-clean-borders = Manuale: pulisci i bordi
aligners-desc-preparation-segmentation-clean-borders = Elimina le piccole isole e leviga il confine irregolare tra denti e gengiva.

aligners-menu-preparation-reconstruction = Ricostruzione dei denti
aligners-desc-preparation-reconstruction = Da ogni corona etichettata nasce un dente solido e chiuso, con il suo asse e il suo centro di resistenza.

aligners-menu-preparation-reconstruction-automatic = Automatica
aligners-desc-preparation-reconstruction-automatic = Costruisce automaticamente tutti i denti 3D.

aligners-menu-preparation-reconstruction-rebuild-tooth = Manuale: ricostruisci un dente
aligners-desc-preparation-reconstruction-rebuild-tooth = Mostra la zona della scansione di un dente, la verifica e ricostruisce solo quel dente.

aligners-menu-preparation-gingiva = Gengiva
aligners-desc-preparation-gingiva = Gengiva deformabile collegata ai denti 3D.

aligners-menu-diagnosis = Diagnosi
aligners-desc-diagnosis = Spazio, affollamento e complessità del caso, misurati sui denti 3D.

aligners-menu-diagnosis-space-analysis = Analisi dello spazio
aligners-desc-diagnosis-space-analysis = Spazio disponibile e spazio richiesto in ciascuna arcata, e il conseguente affollamento o diastema.

aligners-menu-diagnosis-little-index = Indice di irregolarità di Little
aligners-desc-diagnosis-little-index = Somma degli spostamenti dei punti di contatto dei sei denti anteriori inferiori o superiori.

aligners-menu-diagnosis-bolton = Analisi di Bolton
aligners-desc-diagnosis-bolton = Rapporto tra le larghezze dei denti superiori e inferiori (anteriore e totale), per individuare discrepanze dimensionali.

aligners-menu-diagnosis-complexity = Punteggio di complessità
aligners-desc-diagnosis-complexity = Un punteggio unico per la difficoltà del caso, calcolato dai valori misurati.

aligners-menu-diagnosis-edit-values = Manuale: correggi i valori
aligners-desc-diagnosis-edit-values = Correggi a mano i valori della diagnosi.

aligners-menu-planning = Pianificazione
aligners-desc-planning = Posizione finale dei denti, movimenti, suddivisione in allineatori e calendario del trattamento.

aligners-menu-planning-setup = Setup (posizione finale)
aligners-desc-planning-setup = Dove devono arrivare i denti, calcolata entro i limiti delle evidenze, con stripping ed espansione.

aligners-menu-planning-setup-automatic = Automatico
aligners-desc-planning-setup-automatic = Calcola automaticamente la posizione finale dei denti.

aligners-menu-planning-setup-arch-line = Manuale: modifica la linea d'arcata
aligners-desc-planning-setup-arch-line = Mostra e modifica la linea d'arcata su cui si allineano i denti, poi ricalcola.

aligners-menu-planning-movements = Movimenti per dente (6 gradi di libertà)
aligners-desc-planning-movements = Sposta e ruota ogni dente a mano nei sei gradi di libertà: traslazione e rotazione attorno al centro di resistenza.

aligners-menu-planning-staging = Staging
aligners-desc-planning-staging = Il movimento viene diviso in allineatori senza superare il movimento massimo per allineatore.

aligners-menu-planning-staging-automatic = Automatico
aligners-desc-planning-staging-automatic = Calcola automaticamente gli allineatori, con uno staging senza collisioni.

aligners-menu-planning-staging-tooth-speed = Manuale: rallenta un dente
aligners-desc-planning-staging-tooth-speed = Riduce la velocità di un dente nello staging, per esempio un dente che non ha seguito il piano.

aligners-menu-planning-ipr = Stripping (IPR)
aligners-desc-planning-ipr = Riduzione interprossimale: quanto smalto rimuovere su ogni contatto e a quale allineatore.

aligners-menu-planning-attachments = Attacchi
aligners-desc-planning-attachments = Posizione e forma degli attacchi che danno all'allineatore la presa sui denti.

aligners-menu-planning-schedule = Calendario
aligners-desc-planning-schedule = Giorni per allineatore, ore di utilizzo giornaliere e consegna degli allineatori a lotti.

aligners-menu-planning-clinical-review = Revisione clinica
aligners-desc-planning-clinical-review = Le classificazioni proposte dal programma, da confermare o modificare dal clinico prima della produzione.

aligners-menu-occlusion = Occlusione
aligners-desc-occlusion = Relazione tra l'arcata superiore e l'arcata inferiore.

aligners-menu-occlusion-virtual-bite = Morso virtuale
aligners-desc-occlusion-virtual-bite = Stima l'occlusione delle due arcate quando non c'è una scansione del morso.

aligners-menu-occlusion-bite-registration = Registrazione del morso (ICP)
aligners-desc-occlusion-bite-registration = Relazione tra le arcate ricavata dalle scansioni del morso.

aligners-menu-occlusion-bite-registration-icp = Automatica (ICP)
aligners-desc-occlusion-bite-registration-icp = Registra automaticamente le scansioni del morso sulle arcate.

aligners-menu-occlusion-bite-registration-three-points = Manuale: allineamento a 3 punti
aligners-desc-occlusion-bite-registration-three-points = Allinea le scansioni del morso sulle arcate scegliendo tre punti corrispondenti.

aligners-menu-occlusion-contact-map = Mappa dei contatti
aligners-desc-occlusion-contact-map = Mappa a colori della distanza tra le arcate, per vedere i contatti occlusali.

aligners-menu-occlusion-bite-with-aligners = Morso con gli allineatori
aligners-desc-occlusion-bite-with-aligners = Occlusione con gli allineatori indossati: spessore occlusale e contatti.

aligners-menu-occlusion-bite-ramps = Rampe di morso
aligners-desc-occlusion-bite-ramps = Rampe sugli allineatori che aprono il morso, per esempio per muovere gli incisivi.

aligners-menu-production = Produzione degli allineatori
aligners-desc-production = Bordo, spessore e file degli allineatori per la stampante a resina.

aligners-menu-production-base-labels = Base del modello ed etichette
aligners-desc-production-base-labels = Chiude il modello con una base e aggiunge le etichette utili al tecnico.

aligners-menu-production-shell = Guscio dell'allineatore (spessore e gioco)
aligners-desc-production-shell = Il guscio dell'allineatore attorno ai denti: spessore e gioco interno.

aligners-menu-production-shell-automatic = Automatico
aligners-desc-production-shell-automatic = Costruisce il guscio dell'allineatore per ogni stadio.

aligners-menu-production-shell-corrections = Manuale: correzioni
aligners-desc-production-shell-corrections = Corregge a mano la forma di un allineatore.

aligners-menu-production-trim-line = Linea di rifilatura
aligners-desc-production-trim-line = Bordo dell'allineatore: lungo l'equatore del dente o il margine gengivale, con festoni.

aligners-menu-production-stamp = Timbro identificativo
aligners-desc-production-stamp = Codice del caso e data incisi sull'allineatore.

aligners-menu-production-print-check = Controllo di stampa
aligners-desc-production-print-check = Verifica che gli allineatori siano stampabili: superfici chiuse, parete minima, sottosquadri.

aligners-menu-followup = Monitoraggio
aligners-desc-followup = Confronta il trattamento con il piano durante le visite di controllo.

aligners-menu-followup-visits = Visite di controllo
aligners-desc-followup-visits = Confronta una nuova scansione con il piano: denti in pari o in ritardo.

aligners-menu-followup-tracking = Monitoraggio e ripianificazione
aligners-desc-followup-tracking = Nuovo piano a partire dagli allineatori già consegnati.

aligners-menu-output = Esportazione
aligners-desc-output = File e viste del risultato.

aligners-menu-output-export-stl = Esporta STL per stadio
aligners-desc-output-export-stl = Esporta i modelli e gli allineatori di ogni stadio come file STL.

aligners-menu-output-report = Report
aligners-desc-output-report = Relazione clinica e tecnica del caso.

aligners-menu-output-viewer = Visualizzatore paziente e animazione
aligners-desc-output-viewer = Animazione del trattamento allineatore per allineatore, da mostrare al paziente.

aligners-menu-tools = Strumenti
aligners-desc-tools = Strumenti manuali e cronologia delle elaborazioni.

aligners-menu-tools-manual-tools = Strumenti manuali
aligners-desc-tools-manual-tools = Alternative manuali: ogni fase automatica può essere eseguita o corretta a mano.

aligners-menu-tools-history = Cronologia
aligners-desc-tools-history = Fasi fatte, da fare e da rifare; torna a una fase precedente e annulla il ritorno indietro.

## Comandi: etichetta e documentazione di ogni comando registrato (CommandSpec), una
## sezione per fase; aggiungere le chiavi di una fase solo sotto la sua intestazione.
## Chiavi: aligners-cmd-<fase>-<nome> (etichetta) e aligners-cmd-<fase>-<nome>-doc (cosa fa
## e cosa significano i parametri). Un comando che è anche una voce del menu può usare la
## sua chiave aligners-menu-<percorso> come etichetta.

## ---- commands: frame ----

aligners-cmd-frame-world = Riferimento mondo
aligners-cmd-frame-world-doc = Porta il paziente sugli assi del mondo: calcola la trasformazione rigida che sposta il caso in modo che l'origine sia il centro dell'arcata sul piano occlusale, la linea mediana corra lungo Y con gli incisivi verso -Y e la destra del paziente verso -X, l'arcata inferiore abbia i denti in alto (+Z) e la superiore i denti in basso. Le scansioni non vengono modificate: la trasformazione è salvata nel caso. Il riferimento è l'arcata inferiore, oppure la superiore se manca l'inferiore. Il riferimento d'arcata viene dai semi dell'arcata di riferimento se sono abbastanza, altrimenti è stimato dalla forma della scansione. Parametro: reference (auto, upper o lower).

## ---- commands: demo ----

aligners-cmd-demo-generate = Arcata dimostrativa
aligners-cmd-demo-generate-doc = Genera un'arcata dentale sintetica (un typodont, non dati di un paziente) con la verità di riferimento: la scansione è aggiunta al caso come scansione superiore o inferiore, al posto di quella già presente, e il seme di ogni dente e le etichette dei denti per vertice sono salvati nei dati di Allineatori. Gli stessi parametri danno sempre lo stesso modello. Parametri: arch (upper o lower), severity (mild, moderate o severe: quanto i denti sono ruotati, spostati e affollati), seed (numero dell'estrazione casuale), third_molars (include i terzi molari, predefinito false).

## ---- commands: autoseed ----

aligners-cmd-autoseed-orientation-doc = Porta il paziente sugli assi del mondo, come il comando del riferimento mondo, e salva la trasformazione nel caso (le scansioni non vengono modificate): l'origine è il centro dell'arcata sul piano occlusale, gli incisivi verso -Y, la destra del paziente verso -X, l'arcata inferiore con i denti in alto e la superiore con i denti in basso. Il riferimento d'arcata viene dai semi dell'arcata di riferimento se sono abbastanza, altrimenti dalle corone della scansione (direzione occlusale dal bordo aperto o dalla base piatta, direzione anteriore dalla U delle sommità delle corone). Parametro: reference (auto, upper o lower; auto è l'arcata inferiore, altrimenti la superiore).
aligners-cmd-autoseed-numbering-doc = Trova i denti di ogni scansione d'arcata e mette un seme numerato (FDI) sulla faccia occlusale di ogni dente, al posto dei semi di quell'arcata. Il profilo delle corone lungo l'arcata è confrontato con le larghezze tipiche dei denti per ogni numero plausibile di molari; un dente dentro l'arcata può mancare. La proposta va verificata. Non riesce se trova meno di 4 denti: posiziona i semi a mano. Parametri: arch (all, upper o lower), third_molars (possono esserci i terzi molari, predefinito false), missing (numeri FDI dei denti che si sa mancare), ends (dente distale imposto del lato destro e sinistro, posizioni da 5 a 8, per esempio [7, 7]; predefinito scelto dai dati).

## ---- commands: segmentation ----
aligners-cmd-segmentation-automatic = Segmentazione automatica
aligners-cmd-segmentation-automatic-doc = Assegna ogni vertice della scansione a un dente (numero FDI) o alla gengiva (0), partendo dai semi dei denti. La scansione viene portata nel sistema di riferimento dell'arcata definito dai semi e semplificata a circa 30 000 facce; un random walk sul grafo delle facce, guidato dalle pieghe della superficie (Lai 2009) e seminato da un disco attorno a ogni seme e da vincoli gengivali (Zou 2015, Liao 2015), dà la prima separazione, ripetuta con ogni seme spostato al centro della sua corona. Le etichette vengono trasferite alla scansione completa, ripulite (un solo pezzo connesso per dente) e i loro confini rifiniti alla massima risoluzione. Le etichette sono salvate nel caso e sostituiscono le precedenti dell'arcata; le scansioni e i semi non vengono modificati. Richiede i semi. Parametri: arch (upper o lower, predefinito tutte le arcate con semi), target_faces, eta_convex, sigma, refine.

## ---- commands: teeth ----
aligners-cmd-teeth-frames = Assi e larghezze dei denti
aligners-cmd-teeth-frames-doc = Misura ogni dente segmentato: il sistema di riferimento locale (asse mesio-distale, vestibolo-linguale e asse lungo, in coordinate del caso), le larghezze mesio-distale e vestibolo-linguale, l'altezza della corona, la cuspide, i contatti interprossimali e la giunzione amelo-cementizia, la cresta alveolare, l'apice radicolare e il centro di resistenza stimati (stime di popolazione, una scansione di superficie non contiene le radici). L'asse lungo deriva dal margine gengivale e dalle pareti laterali della corona, l'asse mesio-distale dai contatti, dal bordo incisale o dalla simmetria della corona. I sistemi di riferimento sono salvati nel caso e le misure sono restituite; le scansioni non vengono modificate. Richiede la segmentazione. Parametri: arch (all, upper o lower), margin_to_crest_mm, cres_ratio, max_tilt_deg.

## ---- commands: reconstruct ----

## ---- commands: gingiva ----

## ---- commands: diagnosis ----

## ---- commands: setup ----
aligners-cmd-diagnosis-space-analysis-doc = Esegue l'intera diagnosi delle arcate segmentate a partire dai denti misurati e la salva nel caso: lo spazio disponibile tra i contatti mesiali dei primi molari (lunghezza d'arco su un polinomio di quarto grado adattato ai denti, AlHarbi 2008) contro lo spazio richiesto (somma delle larghezze mesio-distali), l'indice di irregolarita di Little, lo spazio maggiore tra le corone, i rapporti di Bolton e il punteggio di complessita CAT-CAT. Restituisce il rapporto; una discrepanza negativa indica affollamento. Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-little-index-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con l'indice di irregolarita di Little (Little 1975): la somma dei cinque spostamenti, nel piano occlusale, tra i punti di contatto anatomici dei sei denti anteriori, con la sua categoria. Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-bolton-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con i rapporti di Bolton (Bolton 1958): larghezze mesio-distali inferiori su superiori, anteriore (da canino a canino, norma 77,2 +- 1,65 %) e totale (da primo molare a primo molare, norma 91,3 +- 1,91 %). Richiede che entrambe le arcate siano misurate. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-complexity-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con il punteggio di complessita: le voci CAT-CAT dell'analisi del modello che la scansione fornisce (affollamento, rotazioni di premolari e incisivi laterali, deviazione della linea mediana, diastema maggiore; Long 2020) e il livello di difficolta del consenso 2025 (Wang 2025). Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).

## ---- commands: staging ----

## ---- commands: schedule ----

## ---- commands: occlusion ----

## ---- commands: bite ----
aligners-cmd-staging-automatic-doc = Divide il movimento di ogni dente, dalla posizione di riposo al setup, in allineatori. Il numero di allineatori di un dente è il più piccolo che mantiene ogni componente sotto il suo limite per allineatore (traslazione 0,25 mm, torque e tip 1 grado, rotazione 2 gradi o 1,5 per canini e premolari) e ogni punto della corona sotto il tetto della corona (0,33 mm); uno stadio parodontale riduce i limiti. Lo schema (proporzionale, velocità massima, distalizzazione sequenziale, strutturato o automatico dalla complessità del caso) decide quando si muove ogni dente. Le corone adiacenti sono controllate a ogni allineatore e il dente che si muove contro il vicino viene ritardato; viene restituito l'allineatore in cui si esegue ogni IPR. Gli allineatori sono salvati nel caso e vengono restituiti le note, le collisioni residue e la sovrapposizione del setup finale. Richiede i sistemi di riferimento dei denti e il setup. Parametri: arch (all, upper o lower), pattern, perio, collision, collision_tol_mm, overcorrection, step_linear_mm, step_vertical_mm, step_rotation_deg, step_rotation_round_deg, step_angular_deg, max_crown_step_mm (ciò che non è indicato viene dalle impostazioni del caso).

## ---- commands: ramps ----
aligners-cmd-schedule-doc = Costruisce il calendario dei cambi degli allineatori con piena aderenza (almeno 22 ore al giorno, cambio alla data prevista): primo e ultimo giorno di ogni allineatore, giorni di utilizzo (uguali per tutti, oppure 7 giorni per le traslazioni anteriori e 14 giorni quando l'allineatore ha movimenti angolari o verticali posteriori), cosa fare prima (attachment, IPR) e gli allineatori passivi che fanno cambiare insieme le due arcate. Il calendario viene restituito e il caso non viene modificato. Richiede lo staging. Parametri: arch (all, upper o lower), start_date (AAAA-MM-GG, predefinita oggi), interval_days, policy (fixed o adaptive), lang (en o it), attachments (denti incollati prima dell'allineatore 1).

## ---- commands: shell ----

## ---- commands: trim ----

## ---- commands: stamp ----

## ---- commands: export ----

## ---- commands: visits ----
aligners-cmd-export-doc = Scrive gli allineatori in una cartella: la scansione di ogni arcata con i denti a ogni stadio come STL binario (upper_stage_00.stl è la situazione iniziale; la gengiva non è ancora deformata), stages.csv con il movimento di ogni dente a ogni stadio e le date dell'allineatore, e visits.ics con i cambi di allineatore delle due arcate come file iCalendar. Il caso non viene modificato. Richiede lo staging. Parametri: dir (obbligatorio), start_date (AAAA-MM-GG, predefinita oggi), interval_days, policy (fixed o adaptive), lang (en o it), title.
