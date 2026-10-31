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

aligners-menu-output-print-check = Controllo di stampabilità
aligners-desc-output-print-check = Verifica che ogni modello e allineatore sia pronto per la stampa: chiuso, manifold, senza auto-intersezioni, parete minima, entro il piano di stampa.

aligners-menu-output-manifest = Manifest
aligners-desc-output-manifest = Hash BLAKE3 e dimensioni dei file di un'esportazione, e loro verifica.

aligners-menu-output-report = Report
aligners-desc-output-report = Relazione clinica e tecnica del caso, scritta con i file di stampa, i programmi del paziente e il manifest dell'esportazione.

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

aligners-cmd-reconstruction-automatic-doc = Chiude ogni dente segmentato in un solido a tenuta. Prima misura i denti (sistemi di riferimento, come Assi e larghezze dei denti); poi taglia ogni corona lungo una linea di margine regolare, ricostruisce la superficie che lo scanner ha chiuso sui contatti, le lamine sottili e la striscia accanto al margine, e chiude l'apertura con una superficie regolare tangente alla corona che si restringe in un colletto sotto il margine gengivale e termina con un moncone arrotondato (fairing a piastra sottile, Kobbelt 1998). La superficie ricostruita segue il contorno convesso di ogni sezione orizzontale della corona e raggiunge la superficie di contatto dove la scansione univa due denti. I solidi sono salvati nel caso (coordinate del caso) e i loro numeri restituiti; le scansioni non cambiano. Richiede la segmentazione. Parametri: arch (all, upper o lower), neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-reconstruction-rebuild-tooth-doc = Chiude di nuovo un dente in un solido a tenuta, con il suo sistema di riferimento salvato (misurato prima se manca), e ne sostituisce il solido nel caso. Richiede la segmentazione. Parametri: fdi (il numero del dente, obbligatorio), neck_depth_mm, apex_mm, neck_scale.

## ---- commands: gingiva ----

aligners-cmd-gingiva-doc = Costruisce la gengiva senza fori di ogni arcata segmentata: la scansione tagliata lungo il margine gengivale regolarizzato di tutti i denti, continuata sotto le corone da una membrana armonica abbassata fino a col_mm (la cresta sotto i denti e il col interdentale visibile quando i denti si allontanano). È salvata nel caso con il dente che ogni vertice segue; pesi di fusione armonici la muovono poi con i denti (margine solidale al suo dente, papille condivise dai denti vicini, base ferma). Le scansioni non cambiano. Richiede la segmentazione. Parametri: arch (all, upper o lower), col_mm, papilla_mm, collar_mm, collar_gap_mm, radius_mm, root_mm, root_fade_mm.

## ---- commands: diagnosis ----

## ---- commands: setup ----
aligners-cmd-setup-automatic-doc = Calcola la posizione finale dei denti (il setup): ogni dente da allineare finisce con le cuspidi prominenti (margine incisale, punta del canino, cuspidi vestibolari) sulla linea d'arcata del paziente, le cuspidi lisciate da una regressione robusta, o sulla linea disegnata dal clinico. Un dente si muove solo se è fuori dalla linea, ruotato o a gradino oltre le tolleranze della sua zona; viene derotato attorno al proprio asse lungo finché è tangente alla linea e livellato solo oltre il gradino tollerato, tip e torque invariati. Lo spazio che manca tra i denti fermi (molari, denti fuori dall'allineatore, denti bloccati) si crea prima con la terapia nell'ordine scelto e solo quanto serve: IPR entro i limiti per superficie, espansione, avanzamento degli incisivi; l'affollamento che resta è distribuito tra i contatti e riportato, con close_spaces gli spazi si chiudono. I denti bloccati tengono la loro posizione finale e fanno da riferimento. Le pose finali e il piano di IPR sono salvati nel caso e viene restituito il riepilogo (denti mossi, IPR per contatto, espansione, avanzamento, affollamento residuo, note). Richiede la segmentazione. Parametri: arch (all, upper o lower), align_scope, priority, ipr_zone, leveling, midline, close_spaces, excluded (denti fuori dall'allineatore), line (linea d'arcata disegnata, coordinate del caso, una sola arcata), tolleranze e limiti (line_sigma_mm, pos_tol_mm, normal_tol_mm, rot_tol_deg, post_normal_tol_mm, post_rot_tol_deg, level_tol_mm, vertical_max_mm, expansion_upper_mm, expansion_lower_posterior_mm, expansion_lower_canine_mm, advance_max_mm, retract_max_mm, ipr_surface_cap_mm, enamel_fraction).
aligners-cmd-diagnosis-space-analysis-doc = Esegue l'intera diagnosi delle arcate segmentate a partire dai denti misurati e la salva nel caso: lo spazio disponibile tra i contatti mesiali dei primi molari (lunghezza d'arco su un polinomio di quarto grado adattato ai denti, AlHarbi 2008) contro lo spazio richiesto (somma delle larghezze mesio-distali), l'indice di irregolarita di Little, lo spazio maggiore tra le corone, i rapporti di Bolton e il punteggio di complessita CAT-CAT. Restituisce il rapporto; una discrepanza negativa indica affollamento. Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-little-index-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con l'indice di irregolarita di Little (Little 1975): la somma dei cinque spostamenti, nel piano occlusale, tra i punti di contatto anatomici dei sei denti anteriori, con la sua categoria. Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-bolton-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con i rapporti di Bolton (Bolton 1958): larghezze mesio-distali inferiori su superiori, anteriore (da canino a canino, norma 77,2 +- 1,65 %) e totale (da primo molare a primo molare, norma 91,3 +- 1,91 %). Richiede che entrambe le arcate siano misurate. Parametri: arch (all, upper o lower).
aligners-cmd-diagnosis-complexity-doc = Esegue l'intera diagnosi (vedi analisi dello spazio) e la restituisce con il punteggio di complessita: le voci CAT-CAT dell'analisi del modello che la scansione fornisce (affollamento, rotazioni di premolari e incisivi laterali, deviazione della linea mediana, diastema maggiore; Long 2020) e il livello di difficolta del consenso 2025 (Wang 2025). Richiede la segmentazione e gli assi dei denti. Parametri: arch (all, upper o lower).

## ---- commands: staging ----

## ---- commands: schedule ----

## ---- commands: occlusion ----
aligners-cmd-occlusion-virtual-bite-doc = Mette l'arcata superiore in occlusione sull'inferiore quando non c'è una scansione del morso: un'occlusione stimata per la visualizzazione, non la relazione registrata sul paziente. L'arcata superiore viene girata di fronte all'inferiore con le linee mediane allineate, gli incisivi superiori davanti agli inferiori di un overjet e i primi molari superiori distali agli inferiori (classe I); piccole rotazioni e spostamenti attorno a questa posizione vengono abbassati fino al primo contatto e si tiene quella con più contatti su entrambi i lati, poi piccole inclinazioni la fanno appoggiare su entrambi i lati e il primo contatto viene misurato esattamente sulle corone. La scansione superiore, le sue etichette, i semi e i sistemi di riferimento dei denti si spostano rigidamente; l'arcata inferiore resta ferma. Servono entrambe le arcate segmentate con i loro semi. Parametri: search (cerca la massima intercuspidazione, predefinito true), overjet_mm, class_i_mm.
aligners-cmd-occlusion-contact-map-doc = Misura i contatti occlusali delle due arcate come sono nel caso: la distanza con segno di ogni vertice delle corone dalle corone dell'altra arcata (negativa dentro una corona, fino a 0,5 mm), i denti in contatto (entro 0,10 mm) con la loro distanza minima, i denti in contatto su ogni lato e la compenetrazione più profonda. Il caso non viene modificato; l'app colora la mappa sui denti. Servono entrambe le arcate segmentate con i loro semi.

## ---- commands: bite ----
aligners-cmd-bite-icp-doc = Registra le due arcate sulle scansioni vestibolari del morso del caso (una o due scansioni con ruolo morso, prese con il paziente in chiusura): ogni scansione del morso viene trovata sulla superficie vestibolare dell'arcata inferiore con una ricerca grossolana e un ICP punto-piano con rifilatura sulle corone, i suoi punti sull'arcata inferiore vengono tolti e l'arcata superiore viene registrata sul resto. L'arcata inferiore resta ferma; le scansioni del morso si spostano su di essa e la scansione superiore, le sue etichette, i semi e i sistemi di riferimento dei denti si spostano su di esse. Restituisce l'adattamento di ogni scansione (area in comune, denti, RMS, coerenza), la concordanza di due scansioni, la compenetrazione e il giudizio con i motivi per verificarla. Nulla viene spostato quando la registrazione è rifiutata (sovrapposizione insufficiente, superfici che non combaciano, arcate che non si fronteggiano o si compenetrano troppo). Servono entrambe le arcate segmentate con i loro semi.
aligners-cmd-bite-three-points-doc = Registra le due arcate sulle scansioni del morso come la registrazione automatica, ma parte per la prima scansione del morso da punti scelti a mano: almeno 3 punti sull'arcata inferiore e i punti corrispondenti sulla scansione del morso (distanti tra loro, ad esempio canino, premolare e molare), facoltativamente anche sull'arcata superiore, tutti in coordinate del caso. Da usare quando la ricerca automatica non riesce. Parametri: lower_points, bite_points, upper_points, bite_points_upper (liste di [x, y, z] in mm).
aligners-cmd-staging-automatic-doc = Divide il movimento di ogni dente, dalla posizione di riposo al setup, in allineatori. Il numero di allineatori di un dente è il più piccolo che mantiene ogni componente sotto il suo limite per allineatore (traslazione 0,25 mm, torque e tip 1 grado, rotazione 2 gradi o 1,5 per canini e premolari) e ogni punto della corona sotto il tetto della corona (0,33 mm); uno stadio parodontale riduce i limiti. Lo schema (proporzionale, velocità massima, distalizzazione sequenziale, strutturato o automatico dalla complessità del caso) decide quando si muove ogni dente. Le corone adiacenti sono controllate a ogni allineatore e il dente che si muove contro il vicino viene ritardato; viene restituito l'allineatore in cui si esegue ogni IPR. Gli allineatori sono salvati nel caso e vengono restituiti le note, le collisioni residue e la sovrapposizione del setup finale. Richiede i sistemi di riferimento dei denti e il setup. Parametri: arch (all, upper o lower), pattern, perio, collision, collision_tol_mm, overcorrection, step_linear_mm, step_vertical_mm, step_rotation_deg, step_rotation_round_deg, step_angular_deg, max_crown_step_mm (ciò che non è indicato viene dalle impostazioni del caso).

## ---- commands: ramps ----
aligners-cmd-bite-with-aligners-doc = Chiude la bocca sui due allineatori di uno stadio e misura il morso con gli allineatori indossati. La superficie esterna di un allineatore e l'offset del modello del gioco piu lo spessore della parete, vista dal lato occlusale; lo spessore occlusale (modo auto, manual o same) rende le superfici che masticano piu sottili delle pareti, in auto lo stesso su entrambi gli allineatori, il piu spesso che porta l'apertura del morso ai primi molari a opening_target_mm, mai sotto min_thickness_mm. L'arcata superiore si sposta dalla relazione delle scansioni al primo contatto degli allineatori, con una rotazione attorno all'asse cerniera dei condili (closing hinge, hinge_posterior_mm dietro e hinge_height_mm sopra il punto incisivo inferiore) o lungo l'asse occlusale (vertical); incisivi e canini si misurano in 3D, gli altri denti su mappe di altezza. Le rampe del caso fanno parte degli allineatori quando sono attive. Con equilibrate si toglie il materiale dei due allineatori dove si compenetrerebbero (mai sotto il minimo, al massimo equilibrate_max_mm) finche equilibrate_teeth_per_side denti si toccano per lato. Restituisce lo spessore, l'apertura ai molari e agli incisivi causata dagli allineatori, i denti in contatto per lato e una frase. Servono le due arcate segmentate con i semi; l'arcata con meno allineatori resta all'ultimo. Le impostazioni usate restano nel caso salvo save falso. Parametri: stage, mode, thickness_mm, min_thickness_mm, opening_target_mm, closing, hinge_posterior_mm, hinge_height_mm, equilibrate, equilibrate_teeth_per_side, equilibrate_max_mm, save.
aligners-cmd-bite-ramps-doc = Rampe di rialzo: piccole protuberanze sulla superficie esterna di un allineatore su cui appoggia l'antagonista (le rampe anteriori sulla faccia palatale degli incisivi superiori aprono il morso dietro nel morso profondo). Azione propose (predefinita): quando l'overbite degli incisivi allo stadio supera overbite_min_mm, una rampa sulla faccia palatale di ogni incisivo superiore centrale e laterale sotto l'allineatore che gli inferiori raggiungono, con la cima girata a meta tra la faccia e il piano occlusale, della dimensione width_mm x depth_mm x height_mm; sostituisce le rampe proposte prima, tiene quelle messe a mano e attiva le rampe. Azione add: una rampa a mano sul dente tooth nel punto point (coordinate del caso del dente a riposo) con la normale uscente. Azione remove: le rampe del dente tooth, tutte senza. Azione set: solo le impostazioni (enabled, dimensioni, overbite_min_mm). Le rampe seguono il dente a ogni stadio; il guscio le unisce all'allineatore e il morso con gli allineatori le conta. Servono le due arcate segmentate con i semi. Parametri: action, stage, enabled, tooth, point, normal, width_mm, depth_mm, height_mm, overbite_min_mm.
aligners-cmd-schedule-doc = Costruisce il calendario dei cambi degli allineatori con piena aderenza (almeno 22 ore al giorno, cambio alla data prevista): primo e ultimo giorno di ogni allineatore, giorni di utilizzo (uguali per tutti, oppure 7 giorni per le traslazioni anteriori e 14 giorni quando l'allineatore ha movimenti angolari o verticali posteriori), cosa fare prima (attachment, IPR) e gli allineatori passivi che fanno cambiare insieme le due arcate. Il calendario viene restituito e il caso non viene modificato. Richiede lo staging. Parametri: arch (all, upper o lower), start_date (AAAA-MM-GG, predefinita oggi), interval_days, policy (fixed o adaptive), lang (en o it), attachments (denti incollati prima dell'allineatore 1).

## ---- commands: shell ----
aligners-cmd-shell-automatic-doc = Costruisce il guscio dell'allineatore di ogni stadio prima del taglio: il modello dello stadio (i denti chiusi nella posizione dello stadio e la gengiva che li segue, chiusa fino a una base piana come un modello in gesso rifilato) con le fessure più strette del doppio di blockout_mm riempite, poi il materiale tra i suoi offset al gioco interno e al gioco più lo spessore della parete (un foglio di spessore uniforme steso sul modello, costruito su campi di distanza con segno). Ogni guscio è una sola superficie chiusa e manifold, ridotta entro la tolleranza delle mesh. I gusci vengono controllati e ne viene restituito il resoconto (triangoli, chiusura, spessore misurato della parete, secondi); nel caso non viene salvato nulla, l'esportazione li ricostruisce. Richiede i denti chiusi e la gengiva. Parametri: arch (all, upper o lower), stages (numeri degli allineatori, predefinito tutti), thickness_mm e clearance_mm (predefiniti dalle impostazioni del caso), model_voxel_mm, shell_voxel_mm, blockout_mm, base_height_mm, tolerance_mm, insertion_axis.
aligners-cmd-shell-model-doc = Costruisce il modello da stampare di ogni stadio: i denti chiusi nella posizione dello stadio e la gengiva che li segue, chiusa fino a una base piana base_height_mm sotto la gengiva come un modello in gesso rifilato (la gengiva esatta vicino ai denti, più lontano un campo di altezze visto dal lato occlusale che non sale mai allontanandosi dai denti), con le fessure più strette del doppio di blockout_mm riempite. Ogni modello è una sola superficie chiusa e manifold, ridotta entro la tolleranza delle mesh; le etichette non vengono ancora aggiunte. I modelli vengono controllati e ne viene restituito il resoconto; nel caso non viene salvato nulla, l'esportazione li ricostruisce. Richiede i denti chiusi e la gengiva. Parametri: come il guscio dell'allineatore.
aligners-cmd-shell-blockout = Modello con blockout
aligners-cmd-shell-blockout-doc = Costruisce il modello con blockout di ogni stadio: il modello da stampare con ogni sottosquadro rispetto all'asse di inserzione riempito (tutto ciò che è nascosto dalla direzione di inserzione, così che una retta lungo l'asse incontri il solido una sola volta). Ogni modello con blockout è una sola superficie chiusa e manifold, ridotta entro la tolleranza delle mesh. I modelli vengono controllati e ne viene restituito il resoconto; nel caso non viene salvato nulla. Richiede i denti chiusi e la gengiva. Parametri: come il guscio dell'allineatore; insertion_axis è la direzione di inserzione in coordinate dell'arcata (predefinita [0, 0, 1], occlusale).

## ---- commands: trim ----
aligners-cmd-trim-line-doc = Calcola la linea di rifilatura dell'allineatore di ogni arcata e taglia il guscio lungo di essa. La linea segue la modalità: equatore (la linea di massima convessità di ogni corona lungo la sua via di inserzione, insertion_axis: vertical, common (un'unica via rigida inclinata come sul parallelometro) o per_tooth (predefinito, ogni dente la sua via), spostata di equator_offset_mm verso la gengiva, con le linguette strette arrotondate, mai sotto il margine gengivale né a meno di 1 mm dalla sommità: nessun sottosquadro e nessuna gengiva coperti), dritta (lo zenit gengivale del lato vestibolare e di quello linguale di ogni dente, extension_mm sotto di esso), festonata (il margine gengivale, extension_mm sotto di esso) o liscia (il margine dei denti coperti lungo l'arcata, levigato con festoni dolci di scallop_mm, margin_gap_mm sopra di esso, attorno alla faccia distale dell'ultimo dente di ogni lato; festonata quando meno di due denti hanno un margine). Le linee sono per dente; il bordo liscio è un'unica linea dell'arcata (arch). Per la linea passa una superficie di rifilatura e il guscio (uno strato dello spessore della parete steso sul modello, con le fessure più strette del doppio di blockout_mm riempite) conserva ciò che sta sopra di essa ed entro la portata delle corone, con il bordo arrotondato di round_mm (predefinito 0,3 volte la parete, al massimo 0,3 mm): un unico solido chiuso e manifold. Finché il modello di uno stadio non arriva dalla fase del guscio, il modello è la scansione segmentata, che deve essere un solido chiuso (l'arcata dimostrativa lo è). Il caso non viene modificato: la linea di ogni dente (coordinate del caso), le dimensioni dell'allineatore, se è chiuso e manifold e i fori nella parete vengono restituiti e, con dir, l'allineatore viene scritto lì come STL binario (upper_trimmed.stl, lower_trimmed.stl). Parametri: arch (all, upper o lower), dir, mode (equator, straight, scalloped o smooth), insertion_axis, equator_offset_mm, extension_mm, margin_gap_mm, scallop_mm (predefiniti le impostazioni di rifilatura del caso), round_mm, thickness_mm, clearance_mm (predefiniti le impostazioni del caso), voxel_mm, blockout_mm.

## ---- commands: stamp ----
aligners-cmd-stamp-doc = Mette la scritta di riconoscimento di un allineatore (arcata, numero dell'allineatore, data e codice del caso, per esempio SUP 03 / 131026 / CODICE) sulla scansione di un'arcata: in rilievo (kind model, predefinito) o incisa (kind shell). La scritta è il contorno di un carattere incluso (B612 Mono Bold) trasformato in un campo di distanza e applicato lungo le normali alla superficie ai triangoli sotto di essa, raffinati a resolution_mm: la mesh mantiene la topologia (una mesh chiusa resta chiusa e manifold) e le etichette. Va su un molare o su un secondo premolare dell'arcata segmentata, trovato come nel plugin (prima il dente scelto, prima la faccia linguale, inclinata verso l'occlusale finché sta, senza il codice del caso se la scritta è troppo grande), oppure in un punto dato a mano in coordinate del caso (placement mode manual, che non richiede la segmentazione). Il caso non viene modificato: il risultato è descritto (dente, faccia, dimensioni della scritta) e, con dir, scritto lì come STL binario (upper_stamped.stl, lower_stamped.stl). Parametri: arch (all, upper o lower), dir, text (un modello: i campi arch, number, passive, date e code vanno tra parentesi graffe), number, passive, code, date (AAAA-MM-GG, predefinita oggi), date_format (DDMMYY, DD.MM.YY o YYMMDD), height (mm), depth (mm), min_wall, margin, clearance, kind (model o shell), placement (un oggetto con mode tooth, più tooth M6R, face lingual e read inside, oppure con mode manual, più point, normal e up), method (local o sdf), resolution_mm.

## ---- commands: export ----

## ---- commands: auto ----
aligners-cmd-auto = Pipeline completa
aligners-cmd-auto-doc = Esegue tutta la pipeline automatica sulle scansioni del caso, con i parametri predefiniti, in ordine: orientamento, semi automatici delle arcate che non ne hanno, segmentazione, corone chiuse (con i sistemi di riferimento dei denti), gengiva, diagnosi, setup, staging e calendario dei cambi; con entrambe le arcate il morso virtuale e la mappa dei contatti; con export_dir l'esportazione. Il primo passo che non riesce ferma la pipeline: l'errore indica il passo e il caso resta com'era. Restituisce ogni passo con i suoi secondi e il suo risultato. Parametri: seeds (missing: solo le arcate senza semi, all: li sostituisce, keep: mai), third_molars, missing (numeri FDI dei denti mancanti), occlusion (predefinito true), start_date (AAAA-MM-GG, predefinita oggi), interval_days, export_dir, full_report (il pacchetto di esportazione completo invece dei modelli, della tabella e del calendario).

## ---- commands: visits ----
aligners-cmd-visits-doc = Analizza una visita di controllo. La scansione di controllo (scan: percorso di un file STL, OBJ o PLY) viene sovrapposta alla situazione prevista dal piano dopo l'allineatore indossato (stage; predefinito l'ultimo allineatore dell'arcata, 0 è la situazione iniziale), ogni dente del piano viene registrato sulla scansione e la posizione reale è confrontata con quella pianificata. Un dente è in pari quando ogni componente della differenza è entro 0,5 mm e 2 gradi; l'efficacia è il movimento ottenuto rispetto a quello pianificato. Se tutti i denti sono in pari il piano prosegue, altrimenti si suggerisce un affinamento e i denti in ritardo hanno una velocità ridotta (lag_speed) nei nuovi allineatori. La scansione, sovrapposta al piano e segmentata con semi sulle corone ottenute (segment, attivo per impostazione predefinita), e le misure di ogni dente sono salvate come visita del caso. Il piano non viene modificato. Richiede la fase dei denti e, per uno stage maggiore di 0, lo staging. Parametri: scan (obbligatorio), arch (necessario se il caso ha entrambe le arcate), date (AAAA-MM-GG, predefinita oggi), stage, linear_mm, angular_deg, lag_speed, segment.
aligners-cmd-tracking-doc = Ripianifica dal punto in cui si trova davvero il trattamento. Gli allineatori già consegnati restano e una nuova serie parte dopo di essi, verso lo stesso setup finale, numerata dal successivo. La posizione di partenza di ogni dente è quella misurata a una visita di controllo (visit: il suo numero, per impostazione predefinita l'ultima visita dell'arcata; i denti in ritardo si muovono con il fattore di velocità ridotto della visita) oppure, senza scansione, la posizione pianificata all'allineatore consegnato stage. L'IPR già eseguita non viene pianificata di nuovo. Sostituisce gli allineatori dopo quelli consegnati per l'arcata e lascia l'altra arcata. Richiede il setup e il piano oppure una visita. Parametri: arch, visit, stage (non insieme a visit), pattern.

aligners-cmd-export-doc = Scrive gli allineatori in una cartella: la scansione di ogni arcata con i denti a ogni stadio come STL binario (upper_stage_00.stl è la situazione iniziale; la gengiva segue i denti quando l'arcata ha la sua gengiva, altrimenti resta ferma), stages.csv con il movimento di ogni dente a ogni stadio e le date dell'allineatore, e visits.ics con i cambi di allineatore delle due arcate come file iCalendar. Il caso non viene modificato. Richiede lo staging. Parametri: dir (obbligatorio), start_date (AAAA-MM-GG, predefinita oggi), interval_days, policy (fixed o adaptive), lang (en o it), title.
aligners-cmd-report-doc = Pacchetto di esportazione completo in una cartella, in parallelo: il modello di ogni stadio e i gusci degli allineatori (file SUP/INF in modelli_termoformatura e stampa_resina, allineatori passivi compresi), il controllo di stampabilità di ogni mesh (chiusa, manifold, senza auto-intersezioni, parete minima, entro il piano di stampa), stages.csv, visits.ics, il programma del paziente e il manifest di stampa di ogni arcata, report.html (diagnosi, setup, IPR, movimenti, staging, calendario, file, controllo di stampabilità, bibliografia; con una vista occlusale e una frontale), print_check.json e manifest.json con l'hash BLAKE3 di ogni file. Il caso non viene modificato. Richiede lo staging. Parametri: dir (obbligatorio), start_date (AAAA-MM-GG, predefinita oggi), interval_days, policy, lang (en o it: lingua della relazione, dei programmi e del calendario), title, workflow (both, thermo o direct), print_check, self_intersections, min_thickness_mm, build_plate_mm, report, embed_renders, threads, generated (AAAA-MM-GG, fissa la data scritta nei file).
aligners-cmd-print-check-doc = Verifica che i modelli di alcuni stadi siano pronti per la stampa: superficie chiusa e manifold, orientamento coerente, un solo pezzo, nessuna auto-intersezione, nessun foro passante, volume positivo, entro il piano di stampa. Non scrive nulla; il caso non viene modificato. Richiede lo staging. Parametri: stages (predefiniti il primo e l'ultimo), self_intersections, build_plate_mm.
aligners-cmd-manifest-doc = Verifica una cartella di esportazione con il suo manifest.json (hash BLAKE3 e dimensione di ogni file, file mancanti e non elencati), oppure con verify=false scrive un manifest dei file trovati. Parametri: dir (obbligatorio), verify.

## ---- commands: manual ----
aligners-cmd-manual-brush-doc = Dipinge o cancella a mano le etichette dei denti sulla scansione, a piena risoluzione. Il bersaglio è l'unione dei vertici (indici dei vertici della scansione), dell'impronta in centre (coordinate del caso, radius_mm) e del tratto che va da centre attraverso i punti di stroke (un'impronta ogni mezzo raggio); view_dir, la direzione in cui guarda chi osserva, esclude la superficie rivolta dall'altra parte. mode paint (predefinito) dà al bersaglio il numero di dente label (0 = gengiva); erase lo riporta alla gengiva (con un'etichetta, solo i vertici che la hanno); fill dà l'etichetta a tutta la corona attorno a vertex (o al vertice più vicino a centre) fino al suo margine, trovata con un random walker sulle pieghe della scansione (con l'etichetta 0 toglie la parte del dente cliccato oltre la corona). only_over limita il cambiamento ai vertici che hanno ora quell'etichetta. Le etichette sostituiscono quelle dell'arcata (tutta gengiva se non era segmentata); un dente la cui regione cambia dopo la costruzione del solido compare in to_rebuild. Parametri: arch, label, mode, vertices, centre, radius_mm, stroke, view_dir, vertex, only_over.
aligners-cmd-manual-clean-doc = Ripulisce le etichette dei denti dopo il pennello: i pezzi staccati di un dente più piccoli di un quarto del suo pezzo principale e i piccoli buchi di gengiva dentro una corona prendono l'etichetta di ciò che li circonda, e il bordo frastagliato viene levigato. Parametri: arch (predefinito ogni arcata segmentata), fdi (un dente, predefinito tutti), rings (anelli di levigatura, da 0 a 3, predefinito 1).
aligners-cmd-manual-renumber-doc = Cambia il numero FDI di un dente; se il nuovo numero è già usato, i due denti si scambiano i numeri. I semi si scambiano i numeri, le etichette dell'arcata vengono rinominate, i denti rinumerati vengono misurati e chiusi di nuovo (il numero ne decide gli assi e il centro di resistenza) e ciò che il setup e lo staging avevano per loro viene scartato. I due numeri devono essere nella stessa arcata. Parametri: src (obbligatorio), dst (obbligatorio).
aligners-cmd-manual-missing-doc = Segna un dente come assente (agenesia, estrazione): il suo seme, le etichette, il riferimento, il solido e i movimenti vengono tolti, il suo spazio resta vuoto e il numero resta nell'elenco dei denti assenti, che i semi e il pennello rispettano. Con missing false il dente è di nuovo presente (da seminare o dipingere, poi ricostruire). Parametri: fdi (obbligatorio), missing (predefinito true).
aligners-cmd-manual-seeds-doc = Aggiunge, sposta o elimina il seme di un dente. Un seme è il centro della faccia occlusale di un dente con il suo numero; la segmentazione parte dai semi, quindi dopo una modifica le etichette dell'arcata sono da rifare e la segmentazione va eseguita di nuovo. Parametri: op (add, move o delete), fdi (obbligatorio), position (coordinate del caso in mm, obbligatoria per aggiungere o spostare), snap_to_scan (mette il seme sul vertice più vicino della scansione).
aligners-cmd-manual-rebuild-tooth = Ricostruisci un dente a mano
aligners-cmd-manual-rebuild-tooth-doc = Costruisce di nuovo un solo dente dalla sua regione sulla scansione: dalle etichette attuali, da una selezione di vertici che diventa il dente (gli altri suoi vertici vanno alla gengiva), oppure dopo aver ritrovato la regione dal seme del dente (resegment). La regione viene controllata prima (dimensioni, altezza, bordo della scansione, buchi) e una non valida viene rifiutata con il motivo, senza cambiare nulla. I denti dell'arcata vengono poi misurati di nuovo e il solido chiuso del dente sostituisce il vecchio; gli altri denti tengono solidi, riferimenti e movimenti, mentre setup e staging di questo dente sono da rifare. Parametri: fdi (obbligatorio), vertices, resegment, neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-manual-adjust-doc = Sposta e ruota a mano un dente, nella posizione finale (setup) o a un allineatore (stage). Il movimento è un incremento nel riferimento del dente, attorno al suo centro di resistenza: mesial_mm, buccal_mm, extrusion_mm, torque_deg, tip_deg, rotation_deg, e/o una posa delta (t in mm, q come quaternione x, y, z, w; absolute la rende la nuova posa; reset riporta prima il dente a riposo). Il risultato è memorizzato come fanno le altre fasi. La corona viene poi controllata contro i vicini con la misura di collisione dello staging: la penetrazione con ogni vicino è riportata prima e dopo, e refuse_on_collision trasforma una collisione in un errore. Una modifica del setup rende lo staging da rifare; lock tiene il dente dove è messo quando il setup viene ricalcolato. Parametri: fdi (obbligatorio), stage, le sei componenti, delta, absolute, reset, lock, refuse_on_collision.
