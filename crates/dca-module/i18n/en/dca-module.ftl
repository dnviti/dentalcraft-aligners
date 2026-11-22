# Aligners module (English, base language).
# Menu keys: aligners-menu-<path>; the path is the command id `aligners.<path>` with the dots
# replaced by hyphens (Fluent ids cannot contain dots). Descriptions: aligners-desc-<path>.

aligners-name = Aligners
aligners-select-hint = Select an item of the Aligners menu.
aligners-status-stub = Not yet available
aligners-status-stub-detail = This function is planned and is not implemented yet.

aligners-menu-case = Case
aligners-desc-case = Bring in the patient's scans, or get a complete plan in one go.

aligners-menu-case-import-scans = Import scans
aligners-desc-case-import-scans = Loads the upper and lower arch scans (STL, PLY, OBJ) into the case without starting any processing.

aligners-menu-auto = Run everything automatically
aligners-desc-auto = Gives you a complete plan from the scans, from finding the teeth to the calendar of the aligners.

aligners-menu-teeth = Teeth
aligners-desc-teeth = Find the teeth on the scans and turn them into 3D teeth that can be moved.

aligners-menu-teeth-find = Find the teeth
aligners-desc-teeth-find = Puts a numbered tooth mark (FDI) on every tooth of each arch, after putting the patient in position.

aligners-menu-preparation-segmentation-automatic = Separate teeth and gum
aligners-desc-preparation-segmentation-automatic = Colours every tooth on the scan, so that each one is told apart from the gum.

aligners-menu-teeth-build = 3D teeth
aligners-desc-teeth-build = Gives you every tooth as a closed 3D model with its axis, and a gum that follows the teeth.

aligners-menu-preparation-numbering-place-seeds = Mark a tooth
aligners-desc-preparation-numbering-place-seeds = Puts a tooth mark on a tooth with the mouse and gives it its FDI number.

aligners-menu-preparation-numbering-delete-seed = Delete a tooth
aligners-desc-preparation-numbering-delete-seed = Removes the tooth mark of a tooth found by mistake; the tooth is not recorded as missing.

aligners-menu-preparation-numbering-renumber = Change a tooth number
aligners-desc-preparation-numbering-renumber = Changes the FDI number of a tooth, or swaps the numbers of two teeth.

aligners-menu-preparation-numbering-mark-missing = Missing tooth
aligners-desc-preparation-numbering-mark-missing = Records that a tooth is missing, so that its space stays empty.

aligners-menu-preparation-segmentation-manual-brush = Tooth brush
aligners-desc-preparation-segmentation-manual-brush = Paints a tooth or the gum by hand on the scan, where the colours are wrong.

aligners-menu-preparation-segmentation-clean-borders = Clean borders
aligners-desc-preparation-segmentation-clean-borders = Removes small stray patches and smooths the border between teeth and gum.

aligners-menu-diagnosis = Diagnosis
aligners-desc-diagnosis = Measure the space, the crowding and the difficulty of the case.

aligners-menu-diagnosis-space-analysis = Case analysis
aligners-desc-diagnosis-space-analysis = Gives you the space analysis, the Little irregularity index, the Bolton ratios and the difficulty of the case.

aligners-menu-planning = Plan
aligners-desc-planning = Decide where the teeth must go and how many aligners it takes.

aligners-menu-occlusion-align = Bite
aligners-desc-occlusion-align = Shows how the two arches meet, from the bite scan when the case has one and as an estimate otherwise, with a map of the contacts.

aligners-menu-planning-setup-automatic = Final position
aligners-desc-planning-setup-automatic = Proposes where every tooth should end up, within safe movement limits.

aligners-menu-planning-movements = Move a tooth
aligners-desc-planning-movements = Moves and turns one tooth by hand, in the final position or at one aligner.
aligners-menu-planning-setup-line = Arch line
aligners-desc-planning-setup-line = Shows the arch line the final position aligns the cusps on and lets you move its points; apply it to compute the final position on it, or go back to the automatic line.
aligners-menu-occlusion-bite-registration-three-points = Three points
aligners-desc-occlusion-bite-registration-three-points = Registers the bite from points picked by hand when the automatic registration fails: 3 points far apart on the lower arch and the same 3 on the bite scan, in the same order; then, if you like, the same for the upper arch.

aligners-menu-planning-aligners = Aligners
aligners-desc-planning-aligners = Divides the movement into a sequence of aligners and gives you their calendar.
aligners-menu-review-classify = Clinical review
aligners-desc-review-classify = The classifications the program proposes from the evidence (case difficulty, aligner pattern, attachments, each tooth), to confirm or change before production.

aligners-menu-production = Production
aligners-desc-production = Get the files for the printer and the study models.

aligners-menu-output-export-all = Export for printing
aligners-desc-output-export-all = Writes to a folder the models and the aligners of every stage, ready to print, with the print check, the report and the list of files.
aligners-menu-production-print = Print
aligners-desc-production-print = Shows which aligners are printed and which are still to print, and opens the print window with them.

aligners-menu-followup = Follow-up
aligners-desc-followup = Compare the treatment with the plan and adjust it.

aligners-menu-followup-visits = Control visit
aligners-desc-followup-visits = Compares a new scan with the plan and tells you which teeth are on track and which are behind.

aligners-menu-followup-tracking = Re-plan
aligners-desc-followup-tracking = Makes a new series of aligners from the last control visit, keeping the ones already delivered.

## Commands: label and documentation of every registered command (CommandSpec), one
## section per stage; append the keys of a stage under its own header only.
## Keys: aligners-cmd-<stage>-<name> (label) and aligners-cmd-<stage>-<name>-doc (what it does
## and what the parameters mean). A command that is also a menu leaf may use its
## aligners-menu-<path> key as label.

## ---- commands: frame ----

aligners-cmd-frame-world = Patient position
aligners-cmd-frame-world-doc = Puts the patient on the axes of the case: computes the move that places the case so that the origin is the centre of the dental arch on the occlusal plane, the midline runs along Y with the incisors towards -Y and the patient's right towards -X, the lower arch has the teeth up (+Z) and the upper arch the teeth down. The scans are not modified: the move is stored in the case. The reference is the lower arch, or the upper arch when there is no lower one. The position of the arch comes from the tooth marks of the reference arch when there are enough of them, otherwise it is estimated from the shape of the scan. A scan whose arch is still provisional is refused: confirm it first. Parameter: reference (auto, upper or lower).

## ---- commands: demo ----

aligners-cmd-demo-generate = Demo arch
aligners-cmd-demo-generate-doc = Generates a synthetic dental arch (a typodont, not patient data) with the ground truth: the scan is added to the case as the upper or lower scan, replacing the one already there, and the tooth mark of every tooth and the tooth colours are stored in the Aligners data. The same parameters always give the same model. Parameters: arch (upper or lower), severity (mild, moderate or severe: how much the teeth are rotated, displaced and crowded), seed (number of the random draw), third_molars (include the third molars, default false).

## ---- commands: autoseed ----

aligners-cmd-autoseed-orientation = Automatic orientation
aligners-cmd-autoseed-orientation-doc = Puts the patient on the axes of the case, like the Patient position command, and stores the move in the case (the scans are not modified): the origin is the centre of the dental arch on the occlusal plane, the incisors towards -Y, the patient's right towards -X, the lower arch with the teeth up and the upper arch with the teeth down. The position of the arch comes from the tooth marks of the reference arch when there are enough of them, otherwise from the crowns of the scan (occlusal direction from the open border or the flat base, anterior direction from the U of the crown tops). Parameter: reference (auto, upper or lower; auto is the lower arch, else the upper).
aligners-cmd-autoseed-numbering = Number the teeth
aligners-cmd-autoseed-numbering-doc = Finds the teeth of each arch scan and puts a numbered tooth mark (FDI) on the occlusal face of every tooth, replacing the tooth marks of that arch. The crown profile along the arch is matched to the typical tooth widths for every plausible number of molars; a tooth inside the arch may be missing. When the result suggests that the scan runs past the last molars (a third molar while they are not looked for, teeth stretched to the limit of the widths, an end tooth on the gum), the teeth are found again with the arch free to end at the last crown, leaving out the gum behind it; the reasons and the first numbering are in diagnostics.checked. An erupted third molar keeps its number. The proposal must be checked. It fails when fewer than 4 teeth are found: mark the teeth by hand. With arch all each arch is numbered on its own: one that fails keeps its tooth marks and is listed in failed with the reason, and the command fails only when every arch fails. What the old tooth marks decided is listed in dropped; the teeth known to be missing are read, not stored. Parameters: arch (all, upper or lower), third_molars (third molars may be present, default false), missing (FDI numbers of the teeth known to be missing), ends (forced distal tooth of the right and left side, positions 5 to 8, e.g. [7, 7]; default chosen by the data).

## ---- commands: segmentation ----
aligners-cmd-segmentation-automatic-doc = Assigns every point of the scan to a tooth (FDI number) or to the gingiva (0), starting from the tooth marks. The scan is moved to the position of its arch and simplified to about 30 000 faces; a first separation follows the folds of the surface from a disk around every tooth mark, and it is repeated with every mark moved to the centre of its crown. The tooth colours are carried to the full scan, cleaned (one connected piece per tooth) and their borders refined at full resolution. They are stored in the case and replace the previous ones of the arch; the scans and the tooth marks are not modified. What was made from the old colours of the arch (3D teeth, gum, measures, analysis, final position, aligners, the doubts of the check) is cleared and listed in invalidated. Needs the tooth marks, and refuses a scan whose arch is still provisional: confirm it first. It also refuses an arch with fewer than 4 tooth marks, or without marks on both sides and on the incisors, because its position cannot be worked out: nothing is changed and the error names the arch. Parameters: arch (upper or lower, default all arches with tooth marks), target_faces (5000 to 200000), eta_convex (0.01 to 1), sigma (0.1 to 10), the three by default from the settings of the case (aligners.settings.segmentation), refine (default true). Returns, per arch, teeth and seconds, and in all teeth, seconds and invalidated.
aligners-cmd-segmentation-settings = Settings of Separate teeth and gum
aligners-cmd-segmentation-settings-doc = Changes the separation parameters stored in the case: target_faces (detail of the simplified scan, 5000 to 200000, default 30000), eta_convex (tooth edge sensitivity, 0.01 to 1, default 0.2), sigma (border smoothing, 0.1 to 10, default 1.0). A field not given stays as it is. They are the values Separate teeth and gum uses when a parameter is not given: from the menu, in Find everything automatically and in the check of the separated teeth. A value outside its range is refused and nothing changes. Returns the three values.
aligners-seg-step-simplify = Simplified scan
aligners-seg-step-first = First separation
aligners-seg-step-transfer = Tooth colours on the full scan
aligners-seg-step-borders = Borders at full resolution
aligners-seg-step-cleanup = Cleanup
aligners-warn-segmentation-doubts = Check the teeth of the { $arch } arch before delivery: the automatic analysis left teeth that are not plausible ({ $list }). Check the tooth marks and their numbers, or correct the teeth by hand.

## ---- commands: teeth ----
aligners-cmd-teeth-frames = Tooth axes and widths
aligners-cmd-teeth-frames-doc = Measures every tooth separated from the gum: the local axes (mesio-distal, bucco-lingual and long axis, in case coordinates), the mesio-distal and bucco-lingual widths, the crown height, the cusp tip, the interproximal contacts and the estimated cemento-enamel junction, alveolar crest, root apex and centre of resistance (population estimates, a surface scan has no roots). The long axis comes from the gingival margin and the side walls of the crown, the mesio-distal axis from the contacts, the incisal edge or the symmetry of the crown. The axes are stored in the case and the measures are returned; the scans are not modified. Needs teeth and gum separated. Parameters: arch (all, upper or lower), margin_to_crest_mm, cres_ratio, max_tilt_deg.

## ---- commands: reconstruct ----

aligners-cmd-reconstruction-automatic = Close all the 3D teeth
aligners-cmd-reconstruction-automatic-doc = Closes every tooth separated from the gum into a watertight solid. The teeth are measured first, as Tooth axes and widths does; each crown is then cut along a smooth margin line, the surface the scanner closed over the contacts, thin sheets and the strip next to the margin are rebuilt, and the opening is closed by a fair surface tangent to the crown that narrows to a neck below the gingival margin and ends in a rounded stump. The rebuilt surface follows the convex outline of every horizontal section of the crown and reaches the contact surface where the scan joined two teeth. The solids are stored in the case (case coordinates) and their numbers returned; the scans are not modified. Needs teeth and gum separated. Parameters: arch (all, upper or lower), neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-reconstruction-rebuild-tooth = Rebuild one 3D tooth
aligners-cmd-reconstruction-rebuild-tooth-doc = Closes one tooth again into a watertight solid, with its stored axes (measured first when missing), and replaces its solid in the case. Needs teeth and gum separated. Parameters: fdi (the tooth number, required), neck_depth_mm, apex_mm, neck_scale.

## ---- commands: gingiva ----

aligners-cmd-gingiva = Gingiva
aligners-cmd-gingiva-doc = Builds the gingiva without holes of every arch with teeth and gum separated: the scan cut along the smoothed gingival margin of all the teeth, continued under the crowns by a smooth membrane lowered up to col_mm (the ridge under the teeth and the interdental col seen when teeth move apart), then closed with a flat base 3 mm under its lowest point along the occlusal axis (the base the import gives an open scan), so that the model is a closed solid. It is stored in the case with the tooth every point moves with; it then moves with the teeth (margin rigid with its tooth, papillae shared by neighbouring teeth, base still). The scans are not modified. Needs teeth and gum separated. Parameters: arch (all, upper or lower), col_mm, papilla_mm, collar_mm, collar_gap_mm, radius_mm, root_mm, root_fade_mm.

## ---- commands: diagnosis ----
aligners-cmd-diagnosis-space-analysis-doc = Runs the whole diagnosis of the arches with teeth and gum separated, from the measured teeth and stores it in the case: the space available between the mesial contacts of the first molars (arc length on a smooth curve fitted to the teeth) against the space required (sum of the mesio-distal widths), the Little irregularity index, the largest space between crowns, the Bolton ratios and the CAT-CAT complexity score. Returns the report; negative discrepancy means crowding. Needs teeth and gum separated and the teeth measured. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-little-index = Little irregularity index
aligners-cmd-diagnosis-little-index-doc = Runs the whole diagnosis (see the space analysis) and returns it with the Little irregularity index: the sum of the five displacements, in the occlusal plane, between the anatomical contact points of the six anterior teeth, with its category. Needs teeth and gum separated and the teeth measured. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-bolton = Bolton analysis
aligners-cmd-diagnosis-bolton-doc = Runs the whole diagnosis (see the space analysis) and returns it with the Bolton ratios: lower over upper mesio-distal widths, anterior (canine to canine, norm 77.2 +- 1.65 %) and overall (first molar to first molar, norm 91.3 +- 1.91 %). Needs both arches to be measured. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-complexity = Complexity score
aligners-cmd-diagnosis-complexity-doc = Runs the whole diagnosis (see the space analysis) and returns it with the complexity score: the CAT-CAT model-analysis items that the scan gives (crowding, rotations of premolars and lateral incisors, midline deviation, largest diastema) and the difficulty level of the published consensus. Needs teeth and gum separated and the teeth measured. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-edit-values = Correct the case analysis by hand
aligners-cmd-diagnosis-edit-values-doc = Writes by hand the values of the case analysis that the clinician measured themselves, for one arch: the discrepancy (space available minus space needed, negative is crowding, -30 to 30 mm), the Little irregularity index (0 to 40 mm), the offset of the dental midline (-10 to 10 mm) and the largest space between crowns (0 to 20 mm). A number writes the value, null goes back to the computed one; a value equal to the computed one is not by hand. The complexity follows the values in force (the CAT-CAT items are worked out again, with the rotations of the teeth), and so do the crowding and the Little index the aligner sequence is planned with. The computed values are kept and brought back by a reset. A value out of its limits, or one that is unknown, refuses the whole command and changes nothing. The aligners already planned are left, out of date: plan them again. A new case analysis of the arch drops the values written by hand. Needs the case analysis. Parameters: arch (upper or lower; default the only arch analysed), values (an object: discrepancy, little, midline_offset, largest_space, each a number or null).

## ---- commands: setup ----
aligners-cmd-setup-automatic-doc = Computes the final position of the teeth (the setup): every tooth to align ends with its prominent cusps (incisal edge, canine tip, buccal cusps) on the patient's own arch line, the cusps smoothed by a robust regression, or on the line drawn by the clinician. A tooth moves only when it is off the line, rotated or stepped beyond the tolerances of its zone; it is derotated about its long axis until it is tangent to the line and levelled only beyond the tolerated step, tip and torque unchanged. The space missing between the fixed teeth (molars, teeth left out of the aligner, locked teeth) is made first by the therapy in the chosen order and only as much as needed: IPR within the limits per surface, expansion, incisor advancement; the crowding left is spread over the contacts and reported, with close_spaces the gaps close. Locked teeth keep their final position and are the reference. The final poses, the IPR plan and the result of each arch (expansion, advancement, residual crowding, notes) are stored in the case and the summary is returned (moved teeth, IPR per contact, expansion, advancement, residual crowding, notes). Needs teeth and gum separated. With both arches separated the bite comes first: when the arches are not related (no bite registration or virtual bite, and the scans not exported in occlusion) the setup refuses and changes nothing. Parameters: arch (all, upper or lower), align_scope, priority, ipr_zone, leveling, midline, close_spaces, excluded (teeth out of the aligner), line (drawn arch line, case coordinates, one arch), tolerances and limits (line_sigma_mm, pos_tol_mm, normal_tol_mm, rot_tol_deg, post_normal_tol_mm, post_rot_tol_deg, level_tol_mm, vertical_max_mm, expansion_upper_mm, expansion_lower_posterior_mm, expansion_lower_canine_mm, advance_max_mm, retract_max_mm, ipr_surface_cap_mm, enamel_fraction). A parameter not given is the one stored in the case, which aligners.settings.setup changes.
aligners-cmd-setup-settings = Settings of Final position
aligners-cmd-setup-settings-doc = Changes the final position parameters stored in the case, the values Final position uses for a parameter it is not given (from the menu, in Find everything automatically, after a drawn arch line). Fields, with the plugin's ranges: align_scope (anterior, premolars, all), line_sigma_mm (1 to 20), normal_tol_mm (0 to 2), rot_tol_deg (0 to 30), post_normal_tol_mm (0 to 2), post_rot_tol_deg (0 to 30), pos_tol_mm (0 to 1), contact_tol_mm (0 to 0.5), fix_molars, align_fixed, fix_third, midline (keep, center, balance), incisor_weight (1 to 20), priority (exp_ipr_pro, ipr_exp_pro, exp_pro_ipr), expansion_upper_mm (0 to 4), expansion_lower_posterior_mm (0 to 4), expansion_lower_canine_mm (0 to 3), advance_max_mm (0 to 4), retract_max_mm (0 to 4), close_spaces, ipr_zone (none, anterior, anterior_premolars, all), ipr_surface_cap_mm (0 to 0.5), enamel_fraction (0.1 to 0.5), leveling (smooth, flat, none), vertical_max_mm (0 to 3), level_tol_mm (0 to 2), level_canines, and the steps of the − and + buttons of Move a tooth: adjust_step_mm (0.01 to 1), adjust_step_deg (0.1 to 10). A field not given stays as it is; a value outside its range is refused and nothing changes. The final position is not computed again. Returns every setting.
aligners-cmd-setup-reset = Back to the initial position
aligners-cmd-setup-reset-doc = Takes every tooth of the final position back to its initial position, like the plugin's reset: the final poses, the IPR plan, the arch line used and the result of the arch go, with the aligners computed from them; the locked teeth stay locked, the arch line drawn by the clinician and the settings stay. Parameters: arch (upper or lower; every arch when absent). Returns what was cleared.
aligners-cmd-setup-line-doc = Sets the arch line of one arch that the final position aligns the prominent cusps on (incisal edges, canine tips, buccal cusps), as drawn by the clinician: the points, in order along the arch, are kept with the case and every later computation of the final position follows them instead of the automatic line; its shape and height decide expansion, advancement and levelling, and the space is made with IPR. Without points the arch goes back to the automatic line. With recompute (default) the final position of the arch is computed again at once with the parameters in force, in the same undo step, and the teeth moved by hand lose their corrections as with any new computation; without it the line is only kept for the next computation. Parameters: arch (upper or lower; default the only arch with tooth colours), points (list of [x, y, z] in mm, case coordinates, at least two), recompute (default true).

## ---- commands: staging ----
aligners-cmd-staging-automatic = Aligner sequence
aligners-cmd-staging-automatic-doc = Divides the movement of every tooth, from its rest position to the setup, into aligners. The number of aligners of a tooth is the smallest one that keeps every component under its limit per aligner (translation 0.25 mm, torque and tip 1 degree, rotation 2 degrees or 1.5 for canines and premolars) and every crown point under the crown cap (0.33 mm); a periodontal stage reduces the limits, and a tooth slowed by hand has them multiplied by its speed. The pattern (proportional, maximum velocity, sequential distalization, structured or automatic from the complexity of the case) decides when each tooth moves. Adjacent crowns are checked at every aligner and the tooth that moves into its neighbour is delayed; the aligner at which each IPR is done is returned. The aligners are stored in the case with the result of each arch (aligners, pattern used, slowed teeth, collisions that remain, overcorrection), and the notes, the collisions that remain and the overlap of the final setup are returned. Needs the teeth measured and the setup. Parameters: arch (all, upper or lower), pattern, perio, collision, collision_tol_mm, overcorrection, step_linear_mm, step_vertical_mm, step_rotation_deg, step_rotation_round_deg, step_angular_deg, max_crown_step_mm (what is not given comes from the settings of the case).
aligners-cmd-staging-settings = Settings of Aligners
aligners-cmd-staging-settings-doc = Changes the aligner and calendar settings stored in the case: the values Aligners uses (from the menu and in Run everything automatically) and the calendar of the export. Nothing is computed again. Fields, with the plugin's ranges: perio (healthy, stage1, stage2, stage3), pattern (auto, proportional, max_velocity, sequential_distal, structured), step_linear_mm (0.05 to 0.5), step_vertical_mm (0.05 to 0.5), step_rotation_deg (0.25 to 4), step_rotation_round_deg (0.25 to 4), step_angular_deg (0.1 to 3), max_crown_step_mm (0.05 to 0.6), collision_check, collision_tol_mm (0 to 0.5), overcorrection, interval_policy (fixed or adaptive), interval_days (5 to 21), start_date (YYYY-MM-DD; empty for the day the calendar is made). A value out of range changes nothing. Returns the settings now in force.
aligners-cmd-tooth-speed = Slow a tooth
aligners-cmd-tooth-speed-doc = Slows one tooth: at each aligner it moves only the given share of its normal limits (0.5 is half the movement per aligner), so it needs more aligners; useful for a periodontal tooth or a difficult movement. When its arch has a final position the aligners of the arch are computed again at once, in the same undo step. Parameters: fdi (a tooth of the case), speed (0.2 to 1, rounded to two decimals; null or 1 is the normal speed). Returns the speed kept and the result of the new aligners.
aligners-cmd-review-classify-doc = Makes the classifications of the clinical review of every arch with a case analysis (parameter arch: all, upper or lower), as one undoable change: per arch the case difficulty from the CAT-CAT score, the aligner pattern, whether aligners alone can treat it; per tooth, when the arch has a final position, the predictability risk of its movement and the attachment it calls for, then for every tooth whether its crown measures are plausible and the type its number names (with a note asking to check the number when the diameters look like another type). Each proposal is preset as the decision and stays to review; the decisions already taken stay where the computed value did not change. With the automatic pattern, when the pattern in force in the review is not the one the aligners of an arch were made with, the aligners of that arch are computed again. Returns the keys of the records, the counts and the attachments of each arch, and the arches made again (restaged).
aligners-cmd-review-decide = Decide a review item
aligners-cmd-review-decide-doc = Records the clinician's decision on one record of the clinical review, as one undoable change. Parameters: key (as lower:attachment:33 or upper:complexity:arch), action (confirm: the proposal, the default; computed: keep the computed value; reset: back to the proposal, to review). Returns the status, the value decided and the next record still to review.
aligners-cmd-review-set-value = Choose a review value
aligners-cmd-review-set-value-doc = Chooses another value for one record of the clinical review, as one undoable change. Parameters: key, value (one of the options of the record). The attachment decided feeds the calendar and the report; the aligner pattern decided is used by the next aligner sequence.
aligners-cmd-review-bulk = Decide all review items
aligners-cmd-review-bulk-doc = Acts on every record of the clinical review at once, as one undoable change. Parameter mode: confirm_unflagged (the default) confirms the proposals still to review that have no note asking for a check; reset brings every record back to its proposal, to review. Returns how many records changed and the counts.

## ---- commands: schedule ----
aligners-cmd-schedule = Aligner calendar
aligners-cmd-schedule-doc = Builds the change calendar of the aligners under full compliance (at least 22 hours a day, changed on the due date): first and last day of every aligner, days of wear (the same for all, or 7 days for anterior translations and 14 days when the aligner has posterior angular or vertical movements), what to do before it (attachments, IPR) and the passive aligners that keep the two arches changing together. The calendar is returned and the case is not modified. Needs the aligner sequence. Parameters: arch (all, upper or lower), start_date (YYYY-MM-DD), interval_days, policy (fixed or adaptive), lang (en or it), attachments (teeth bonded before aligner 1); the start date, days and policy not given are those of the case settings (no start date: today).

## ---- commands: occlusion ----
aligners-cmd-occlusion-virtual-bite = Virtual bite
aligners-cmd-occlusion-virtual-bite-doc = Puts the upper arch in occlusion on the lower one when there is no bite scan: an estimated occlusion for display, not the relation registered on the patient. The upper arch is turned to face the lower one with the midlines aligned, the upper incisors in front of the lower ones by the overjet and the upper first molars distal to the lower ones (Class I); small turns and shifts around that pose are lowered to the first contact and the one with the most contacts on both sides is kept, then small tilts let it rest on both sides and the first contact is measured exactly on the crowns. The upper scan moves as one piece with its tooth colours, tooth marks and tooth axes; the lower arch stays in place. Needs both arches with teeth and gum separated. Parameters: search (search the maximum intercuspation, default true), overjet_mm, class_i_mm.
aligners-cmd-occlusion-contact-map = Contact map
aligners-cmd-occlusion-contact-map-doc = Measures the occlusal contacts of the two arches as they are in the case: the distance of every crown point to the crowns of the other arch (negative inside a crown, up to 0.5 mm), the teeth in contact (within 0.10 mm) with their smallest distance, the teeth in contact on each side and the deepest interpenetration. The case is not modified; the app colours the map on the teeth. Needs both arches with teeth and gum separated.

## ---- commands: bite ----
aligners-cmd-bite-icp = Bite from the bite scans
aligners-cmd-bite-icp-doc = Registers the two arches to the buccal bite scans of the case (one or two scans with the role bite, taken with the patient biting): each bite scan is found on the buccal surface of the lower arch by a coarse search and a fine fit on the crowns, its points on the lower arch are removed and the upper arch is registered to the rest. The lower arch stays in place; the bite scans move onto it and the upper scan, with its tooth colours, tooth marks and tooth axes, moves onto them. Returns the fit of every scan (overlap area, teeth, RMS, coherence), the agreement of two scans, the interpenetration and the verdict with the reasons to review it. Nothing is moved when the registration is rejected (too little overlap, surfaces that do not match, arches that do not face each other or interpenetrate too much). Needs both arches with teeth and gum separated.
aligners-cmd-bite-three-points-doc = Registers the two arches to the bite scans like the automatic registration, but starts the first bite scan from points picked by hand: at least 3 points on the lower arch and the matching points on the bite scan (far apart, for example canine, premolar and molar), optionally also on the upper arch, all in case coordinates. Use it when the automatic search fails. Parameters: lower_points, bite_points, upper_points, bite_points_upper (lists of [x, y, z] in mm).
aligners-cmd-bite-correct = Correct the interpenetration
aligners-cmd-bite-correct-doc = Removes the interpenetration of the arches in the case, as the bite adjustment of intraoral scanner software (the teeth under bite force move in their periodontal ligament): the upper arch, with its tooth colours, tooth marks, tooth axes, 3D teeth and gingiva, moves rigidly along the occlusal axis of the lower arch until the first contact, as with the teeth unloaded. The scans are not reshaped and the bite scans stay where they are. The opening is added to the report of the bite, which keeps the interpenetration the registration gave and gives the one left and the contacts after the move. Refused when the arches do not interpenetrate (at most 0.021 mm to the first contact). Needs both arches with teeth and gum separated. Returns opening_mm, penetration_mm, contact_points, first_contact (upper and lower tooth) and upper_move. No parameters.

## ---- commands: components ----
aligners-cmd-components-transform = Move, turn or scale a 3D component
aligners-cmd-components-transform-doc = Moves, turns or scales one component of the case, as the gizmo of the 3D view does, as one undo step. component: tooth (give fdi), gingiva (give arch) or scan (give arch, or scan_id for any scan, a bite scan included). A tooth moves and turns in the plan: its pose in the final position, or at the aligner given by stage (1 or more), stored as { aligners-menu-planning-movements } stores it, with the collisions with its neighbours in the result; scale changes the size of its 3D tooth about its centre (0.5 to 2 times the size it was built with) until the tooth is built again. The gingiva or the scan of an arch moves the whole arch (its scan, tooth colours, tooth marks, 3D teeth and gingiva) as the bite registration does; a bite scan moves alone; neither can be scaled. move: t in mm and q as a quaternion x, y, z, w, in case coordinates, after what the component already has; absolute: move and scale say where the component ends up from where it started; reset: back to where it started first. The case keeps the total move of each scan and the size of each tooth.

## ---- commands: ramps ----
aligners-cmd-bite-with-aligners = Bite with aligners
aligners-cmd-bite-with-aligners-doc = Closes the mouth on the two aligners of a stage and measures the bite with them worn. The outer surface of an aligner is the offset of the model by the clearance and the wall thickness, seen from the occlusal side; the occlusal thickness (mode auto, manual or same) makes the surfaces that chew thinner than the walls, in auto the same on both aligners, the thickest that brings the opening of the bite at the first molars to opening_target_mm, never under min_thickness_mm. The upper arch is moved from the relation of the scans to the first contact of the aligners, by a rotation about the hinge axis of the condyles (closing hinge, hinge_posterior_mm behind and hinge_height_mm above the lower incisal point) or along the occlusal axis (vertical); the incisors and canines are measured in 3D, the other teeth on height maps. The ramps of the case are part of the aligners when they are enabled. With equilibrate the material of both aligners where they would interpenetrate is ground off (never under the minimum, at most equilibrate_max_mm) until equilibrate_teeth_per_side teeth touch on each side. Returns the thickness, the opening at molars and incisors caused by the aligners, the teeth in contact on each side and a sentence (Italian). Needs both arches with teeth and gum separated; the arch with fewer aligners stays at its last. The settings used are kept in the case unless save is false. Parameters: stage, mode, thickness_mm, min_thickness_mm, opening_target_mm, closing, hinge_posterior_mm, hinge_height_mm, equilibrate, equilibrate_teeth_per_side, equilibrate_max_mm, save.
aligners-cmd-bite-ramps = Bite ramps
aligners-cmd-bite-ramps-doc = Bite ramps: small bumps on the outer surface of an aligner that the antagonist rests on (anterior ramps on the palatal face of the upper incisors open the bite at the back in a deep bite). Action propose (the default): when the overbite of the incisors at the stage is beyond overbite_min_mm, a ramp on the palatal face of every upper central and lateral incisor under the aligner that the lower incisors reach, its top turned half way between the face and the occlusal plane, of the size width_mm x depth_mm x height_mm; it replaces the ramps proposed before and keeps the ones placed by hand, and turns the ramps on. Action add: a ramp by hand on tooth at point (case coordinates of the tooth at rest) with the outward normal. Action remove: the ramps of tooth, all of them without it. Action set: only the settings (enabled, sizes, overbite_min_mm). The ramps follow their tooth at every stage; the shell unites them with the aligner and the bite with the aligners counts them. Needs both arches with teeth and gum separated. Parameters: action, stage, enabled, tooth, point, normal, width_mm, depth_mm, height_mm, overbite_min_mm.

## ---- commands: production ----
aligners-cmd-production-settings = Production settings
aligners-cmd-production-settings-doc = Changes the production settings stored in the case, the values the production commands use for a parameter they are not given. One object per group, only the fields to change: trim (mode: equator, straight, scalloped or smooth; margin_gap_mm -2 to 5; recess_mm 0.6 to 2.5; scallop_mm 0 to 4; equator_offset_mm -2 to 1.5; extension_mm 0 to 4; insertion_axis: vertical, common or per_tooth), shell (shell_thickness_mm 0.3 to 2; clearance_mm 0 to 0.3), stamp (enabled; tooth: M6R, M6L, M7R, M7L, PM5R or PM5L; face: lingual or buccal; read: inside or outside; date: production or start; date_format: DDMMYY, DD.MM.YY or YYMMDD; code: at most 8 letters, digits and -, never the name of the patient; height_mm 0.3 to 2.5; depth_mm 0.02 to 0.3; min_wall_mm 0.2 to 1), bite (the bite with the aligners worn; mode: auto, manual or same; thickness_mm 0.3 to 2; min_thickness_mm 0.3 to 1.5; opening_target_mm 0 to 5; closing: hinge or vertical; hinge_posterior_mm 40 to 120; hinge_height_mm 0 to 80; equilibrate; equilibrate_teeth_per_side 1 to 6; equilibrate_max_mm 0.05 to 1), ramps (enabled; width_mm 0.5 to 8; depth_mm 0.5 to 6; height_mm 0.2 to 4; overbite_min_mm 1 to 8) and quality (draft, light, standard or high). A value outside its range, or a case code that cannot be stamped, changes nothing. Returns every group with its values.
aligners-cmd-production-aligner-end = Where the aligner ends
aligners-cmd-production-aligner-end-doc = Where the aligner of an arch ends on each side of the patient. Action set: the aligner ends with tooth fdi on its side (the arch and the side are those of the tooth, which must be a tooth of the arch): the teeth behind it are not covered and stay where they are, the final position keeps them at rest, the shell, the trim line, the identification text and the bite with the aligners worn leave them out. Action clear: the aligner covers every tooth again, of the arches in arch (all, upper, lower) and on side (right, left; both when missing). Action suggest (the default): changes nothing. Every action returns, for each arch concerned: its end, the teeth left out, the suggested ends (behind a missing premolar or first or second molar with covered teeth behind it, end at the tooth before the space: the shell behind a space is weak) and setup_outdated with a warning when its final position was computed with another end (compute the final position and the aligners again).

## ---- commands: shell ----
aligners-cmd-shell-automatic = Aligner shell
aligners-cmd-shell-automatic-doc = Builds the aligner shell of every stage before the trim: the model of the stage (the closed teeth at their stage position and the gingiva moved with them, closed down to a flat base like a trimmed plaster cast) with the crevices narrower than twice blockout_mm filled, then the material between its offsets at the clearance and at the clearance plus the wall thickness (a sheet of uniform thickness laid on the model). Every shell is one closed surface reduced within the surface tolerance. The shells are checked and their report returned (triangles, closed, wall thickness measured, seconds); nothing is stored in the case, the export builds them again. Needs the closed teeth and the gingiva. Parameters: arch (all, upper or lower), stages (aligner numbers, default all), thickness_mm and clearance_mm (default the case settings), model_voxel_mm, shell_voxel_mm, blockout_mm, base_height_mm, tolerance_mm, insertion_axis.
aligners-cmd-shell-model = Printable model
aligners-cmd-shell-model-doc = Builds the printable model of every stage: the closed teeth at their stage position and the gingiva moved with them, closed down to a flat base base_height_mm below the gingiva like a trimmed plaster cast (the exact gingiva near the teeth, a height field seen from the occlusal side farther away, never rising away from the teeth), with the crevices narrower than twice blockout_mm filled. Every model is one closed surface reduced within the surface tolerance; the stamp is not added yet. The models are checked and their report returned; nothing is stored in the case, the export builds them again. Needs the closed teeth and the gingiva. Parameters: as the aligner shell.
aligners-cmd-shell-blockout = Blockout model
aligners-cmd-shell-blockout-doc = Builds the blockout model of every stage: the printable model with every undercut relative to the insertion axis filled (everything hidden from the insertion direction, so that a line along the axis meets the solid once). Every blockout model is one closed surface reduced within the surface tolerance. The models are checked and their report returned; nothing is stored in the case. Needs the closed teeth and the gingiva. Parameters: as the aligner shell; insertion_axis is the insertion direction in arch coordinates (default [0, 0, 1], occlusal).

## ---- commands: trim ----
aligners-cmd-trim-line = Trim line
aligners-cmd-trim-line-doc = Computes the trim line of the aligner of every arch and cuts the shell along it. The line follows the mode: equator (the height of contour of every crown along its insertion path, insertion_axis: vertical, common (one rigid path tilted as on a surveyor) or per_tooth (the default, every tooth its own path), moved by equator_offset_mm towards the gingiva, its narrow tabs rounded, never under the gingival margin nor within 1 mm of the top: no undercut and no gingiva covered), straight (the gingival zenith of the buccal and of the lingual side of every tooth, extension_mm below it), scalloped (the gingival margin, extension_mm below it) or smooth (the margin of the covered teeth along the arch, smoothed with gentle scallops of scallop_mm, margin_gap_mm above it, round the distal face of the last tooth of each side; scalloped when fewer than two teeth have a margin). The lines are keyed by tooth; the smooth border is one line of the arch (arch). A trim surface goes through the line, and the shell (a sheet of the wall thickness laid on the model with the crevices narrower than twice blockout_mm filled) keeps what is above it and within reach of the crowns, with a rim rounded by round_mm (default 0.3 x the wall, at most 0.3 mm): one closed solid. Until the model of a stage comes from the shell stage, the model is the scan, which must be a closed solid (the demo arch is). The case is not modified: the line of every tooth (case coordinates), the size of the aligner, whether it is closed and the holes through its wall are returned and, with dir, the aligner is written there as binary STL (upper_trimmed.stl, lower_trimmed.stl). Parameters: arch (all, upper or lower), dir, mode (equator, straight, scalloped or smooth), insertion_axis, equator_offset_mm, extension_mm, margin_gap_mm, scallop_mm (default the trim settings of the case), round_mm, thickness_mm, clearance_mm (default the case settings), voxel_mm, blockout_mm.

## ---- commands: stamp ----
aligners-cmd-stamp = Identification stamp
aligners-cmd-stamp-doc = Puts the identification text of an aligner (arch, aligner number, date and case code, for example SUP 03 / 131026 / CODE) on the scan of an arch: embossed (kind model, the default) or engraved (kind shell). The text is the outline of a bundled font, applied along the surface to the triangles under it, which are refined to resolution_mm: a closed surface stays closed and the tooth colours are kept. It goes on a molar or second premolar of the arch (the tooth chosen first, the lingual face first, tilted towards the occlusal side until it fits, the case code dropped if the text is too big), or at a point given by hand in case coordinates (placement mode manual, which needs no teeth and gum separated). The case is not modified: the result is described (tooth, face, size of the text) and, with dir, written there as binary STL (upper_stamped.stl, lower_stamped.stl). Parameters: arch (all, upper or lower), dir, text (a template: the fields arch, number, passive, date and code go in curly braces), number, passive, code, date (YYYY-MM-DD, default today), date_format (DDMMYY, DD.MM.YY or YYMMDD), height (mm), depth (mm), min_wall, margin, clearance, kind (model or shell), placement (an object with mode tooth, plus tooth M6R, face lingual and read inside, or with mode manual, plus point, normal and up), method (local or sdf), resolution_mm.

## ---- commands: export ----
aligners-cmd-export = Export moved scans and calendar
aligners-cmd-export-doc = Writes the aligners into a directory: the scan of each arch with the teeth at every stage as binary STL (upper_stage_00.stl is the initial situation; the gingiva follows the teeth when the arch has its gingiva, else it stays where it is), stages.csv with the movement of every tooth at every stage and the dates of the aligner, and visits.ics with the aligner changes of both arches as an iCalendar file. The case is not modified. Needs the aligner sequence. It writes no printable model, aligner, print check or report: for those see Export for printing. Parameters: dir (required), start_date (YYYY-MM-DD), interval_days, policy (fixed or adaptive), lang (en or it), title; the start date, days and policy not given are those of the case settings (no start date: today).
aligners-cmd-report = Full export package
aligners-cmd-report-doc = Full export package into a directory, in parallel: the model of each stage and the aligner shells (SUP/INF files in modelli_termoformatura and stampa_resina, passive aligners included), the print check of every file (closed, no self-intersections, minimum wall, fits the build plate), stages.csv, visits.ics, the patient programme and the print manifest of each arch, report.html (diagnosis, set-up, IPR, movements, aligner sequence, calendar, files, print check, references; with an occlusal and a frontal render), print_check.json and manifest.json with the checksum of every file. When the teeth are not closed the models are the moved scans and there is no aligner; shells true refuses that. The case is not modified. Needs the aligner sequence. Parameters: dir (required), start_date (YYYY-MM-DD), interval_days, policy (the start date, days and policy not given are those of the case settings; no start date: today), lang (en or it: language of the report, the programmes and the calendar), title, workflow (both, thermo or direct), print_check, self_intersections, min_thickness_mm, build_plate_mm, report, embed_renders, threads, generated (YYYY-MM-DD, fixes the date written in the files).
aligners-cmd-print-check = Print check
aligners-cmd-print-check-doc = Checks that the models of some stages are ready to print: closed surface, consistent orientation, one piece, no self-intersections, no hole through the wall, positive volume, fits the build plate. Writes nothing; the case is not modified. Needs the aligner sequence. Parameters: stages (default the first and the last), self_intersections, build_plate_mm.
aligners-cmd-manifest = List of exported files
aligners-cmd-manifest-doc = Verifies an export folder against its manifest.json (checksum and size of every file, missing and unlisted files), or with verify=false writes a manifest of the files found. Parameters: dir (required), verify.

## ---- commands: print ----
aligners-cmd-production-print-doc = The aligners to print, for the print window, without changing the case: every aligner of the series of each arch (active ones, then the passive ones up to the longer series), each with its id (SUP-03, INF-12-P for a passive one), name, arch, number, passive flag, the file of the last export for printing (null when it is not there) and its status (missing, queued, sent, printed, failed, reprint). Output: print_parts, export_dir (the folder of the last export, or null), printed, missing. In the app it opens the print window with them; an agent gives their files to print.prepare. Parameters: arch (all, upper, lower).
aligners-cmd-print-parts = List the aligners to print
aligners-cmd-print-parts-doc = Lists every aligner of the series with its id (SUP-03, INF-12-P for a passive one), name, arch, number, passive flag, the file in stampa_resina of the last export for printing (null when it is not there) and its print status (missing when it was never marked; record: when, by whom, job, reason). The case is not modified. Output: parts, export_dir. Parameters: arch (all, upper, lower).
aligners-cmd-print-status = Count the printed aligners
aligners-cmd-print-status-doc = Counts the aligners of the series: total, printed, missing (every aligner not printed yet), by_status (missing, queued, sent, printed, failed, reprint), printed_ids and missing_ids. The case is not modified. Parameters: arch (all, upper, lower).
aligners-cmd-print-mark = Mark aligners as printed
aligners-cmd-print-mark-doc = Marks aligners of the series as one undoable change, all or none: an aligner whose status cannot change that way refuses the whole mark. Allowed: to queued from missing, failed or reprint; to sent from anything but printed; to failed from queued or sent; to reprint from printed or failed, by the user; to printed from anything but printed by the user, or from sent by the printer, never by the app. Output: marked (the aligners as aligners.print.parts lists them), summary (as aligners.print.status). Parameters: ids (like SUP-03, INF-12-P), status (queued, sent, printed, failed, reprint), by (user, printer, app; default user), job (the plate file or the printer's job), reason.

## ---- commands: view ----
aligners-cmd-view-aligner = Aligner in the 3D view
aligners-cmd-view-aligner-doc = Shows or hides the aligner over the teeth in the 3D view (show true or false). Shown, every arch with aligners gets the aligner of the stage given (stage: 0 is the initial position; an arch past its last aligner takes its last one) when it has none up to date there, built as the print file is (the shell of the wall on the model of the stage, the ramps, the trim, the occlusal shape) at the detail of the view (the preview detail of the quality, at most 0.45 of the wall thickness) and reduced within the preview tolerance, without the stamp and the print check. The other stages show the nearest aligner built, moved with the teeth and the gingiva. A change of the design, the teeth or the plan leaves the aligners built before out of date: they are no longer drawn and the next build replaces them. Hidden, every aligner built for the view is dropped. Both arches are built at the same time. Output: built (arch, stage, faces, seconds), kept (the arches already up to date), the aligners kept for the view and the notes of the build.
aligners-cmd-view-aligner-exact = Exact aligner of a stage
aligners-cmd-view-aligner-exact-doc = Builds the aligner of a stage exactly as the export writes it (the provider and the choice of aligners.output.export-all: the quality, the stamp and its code of the case) for every arch with aligners, keeps it for the 3D view and shows the aligner. Parameters: stage (0 is the initial position; an arch past its last aligner takes its last one), quality (draft, light, standard, high; default the quality of the case). An exact aligner already built for the same stage and quality is not built again. Output as aligners.view.aligner.

## ---- commands: auto ----
aligners-cmd-auto-doc = Runs the whole automatic pipeline on the scans of the case, with the default parameters, in order: the arch of the provisional scans, automatic tooth marks of the arches that have none, orientation from the tooth marks, teeth and gum, the check of the separated teeth of each arch (the tooth marks proposed in this run are corrected and the teeth and gum of the arch separated again, those placed by hand are only checked), 3D teeth (with their measures), gingiva, diagnosis; with both arches not yet related, the bite before the final position (the registration on the bite scan when the case has one, otherwise the virtual bite, reported as a warning); setup, aligner sequence, the clinical review (proposals to review, the decisions already taken kept) and change calendar; when the arch was decided by the shape and the width of its teeth alone, the crown widths (incisor / molar ratio and width fit) verify it after the 3D teeth, and the pipeline starts again from the numbering if the arch changes, at most three builds; with both arches the contact map; with export_dir the export. The first step that fails stops the pipeline: the error names the step and the case is left as it was, and an arch that is still a guess stops it with the advice to confirm it. Returns every step with its seconds and its output. Parameters: seeds (missing: only the arches without tooth marks, all: replace them, keep: never), third_molars, missing (FDI numbers of missing teeth), occlusion (default true; false leaves out the bite and the contact map, and two arches not related then stop the run at the setup), start_date (YYYY-MM-DD, default the one of the case settings, else today), interval_days, export_dir, full_report (the full export package instead of the models, the table and the calendar).

## ---- commands: wrappers ----
aligners-cmd-teeth-find-doc = Finds the teeth of each arch and puts the patient in position, as one undoable change: the numbering of the teeth, then Automatic orientation from the new tooth marks, the same two steps Run everything automatically starts with, so the position of the patient is the same. Each arch is numbered on its own: an arch where the teeth are not found keeps what it had and is listed in failed with the reason, the other keeps its new tooth marks; it fails only when no arch works. The new tooth marks replace the old ones of the arch; when they differ, what was computed from the old ones (tooth colours, 3D teeth, gum, analysis, final position, aligners) is cleared and has to be done again. The proposal must be checked. Returns every step with its output. Run from the menu it gets third_molars and missing from the settings of the case (aligners.settings.find-teeth); called as a command it takes what it is given. Parameters: arch (all, upper or lower), third_molars (third molars may be present, default false), missing (FDI numbers of the teeth known to be missing, added to the ones marked missing in the case), ends (forced distal tooth of the right and left side, positions 5 to 8, e.g. [7, 7]; default chosen by the data).
aligners-cmd-occlusion-align-doc = Puts the two arches in occlusion and measures the contacts, as one undoable change: Bite from the bite scans when the case has a bite scan, otherwise Virtual bite, then Contact map. A registration that is refused stops the command with its reason: the estimate does not take over. Needs both arches with teeth and gum separated. Returns method (bite_scan or virtual_bite, which one ran) and every step with its output. No parameters.
aligners-cmd-planning-aligners-doc = Divides the movement into aligners and builds their calendar, as one undoable change: Aligner sequence with the settings of the case and its slowed teeth, then Aligner calendar. Needs the teeth measured and the final position. Returns every step with its output. Parameters: arch (all, upper or lower), start_date (YYYY-MM-DD), interval_days, policy (fixed or adaptive); what is not given comes from the settings of the case (no start date: today).
aligners-cmd-export-all-doc = Everything the printer needs, into a folder: the printable model of each stage on its flat base with the identification stamp, the aligner of each stage trimmed along the trim line, with the bite ramps and the occlusal thickness of the case and the stamp engraved (the SUP and INF files in modelli_termoformatura and stampa_resina, passive aligners included), the print check of every file (print_check.json), stages.csv, visits.ics, the patient programme and the print list of each arch, report.html and manifest.json with the checksum of every file. It is Full export package with the closed teeth required: when an arch has no 3D teeth it stops and says so, instead of exporting the moved scans. The case is not modified. Needs the aligner sequence and the 3D teeth. Returns what Full export package returns. Parameters: dir (required), start_date (YYYY-MM-DD, default the one of the case settings, else today), interval_days, quality (draft, light, standard or high; default standard, the production quality), lang (en or it, default the language of the app), stamp_code.

## ---- commands: visits ----
aligners-cmd-visits-doc = Analyses a control visit. The progress scan (scan: path of an STL, OBJ or PLY file) is placed on the situation the plan expects after the aligner stage the patient wears (default: the last aligner of the arch; 0 is the initial situation), every tooth of the plan is registered to the scan, and the position it really has is compared with the planned one. A tooth is on track when every component of the difference is within 0.5 mm and 2 degrees; the efficacy is the achieved over the planned movement. If every tooth is on track the plan goes on, otherwise a refinement is suggested and the teeth that are behind get a lower velocity (lag_speed) in the new aligners. The scan, placed on the plan with teeth and gum separated from tooth marks on the achieved crowns (`segment`, on by default), and the figures of every tooth are stored as a visit of the case. The scan keeps the units of the file: it is not rescaled, and a size that fits another unit gives a units_suspect warning; the warnings of the file (code, key, args, message) are in the output as warnings. The plan is not modified. Needs the 3D teeth and, for a stage above 0, the aligner sequence. Parameters: scan (required), arch (default the only arch of the case; with both arches, the one recognised from the name and the shape of the file), date (YYYY-MM-DD, default today), stage, linear_mm, angular_deg, lag_speed, segment.
aligners-cmd-tracking-doc = Re-plans from where the treatment really is. The aligners already delivered are kept and a new series starts after them, towards the same final setup, numbered from the next aligner on. The starting position of every tooth is the one measured at a control visit (visit: its number, the last visit of the arch by default; the teeth that were behind move at the visit's lower velocity factor) or, without a scan, the planned position at the delivered aligner stage. The IPR already done is not planned again. Replaces the aligners after the delivered ones for the arch and keeps the other arch. Needs the setup and the plan or a visit. Parameters: arch, visit, stage (not together with visit), pattern.

## ---- commands: manual ----
aligners-cmd-manual-brush-doc = Paints or erases tooth colours on the scan by hand, at full resolution. The target is the union of `vertices` (point indices of the scan), the dab at `centre` (case coordinates, `radius_mm`) and the stroke that goes from `centre` through the points of `stroke` (a dab every half radius); `view_dir`, the direction the viewer looks along, leaves out the surface that looks away from the viewer. mode paint (default) gives the target the tooth number `label` (0 = gingiva); erase sends it back to the gingiva (with a `label`, only the points of that tooth); fill gives the tooth number to the whole crown around `vertex` (or the point nearest to `centre`) up to its margin, found from the folds of the scan (with 0 it takes away the part of the clicked tooth beyond its crown). `only_over` restricts the change to the points of that tooth now. The tooth colours replace those of the arch (all gingiva when teeth and gum were not separated); a tooth whose region changes after its solid was built is listed in to_rebuild. Parameters: arch, label, mode, vertices, centre, radius_mm, stroke, view_dir, vertex, only_over.
aligners-cmd-manual-clean-doc = Cleans the tooth colours after the brush: the detached pieces of a tooth smaller than a quarter of its main piece and the small holes of gingiva inside a crown take the colour of their surroundings, and the jagged border is smoothed. Parameters: arch (default every arch with teeth and gum separated), fdi (one tooth, default all), rings (smoothing rings, 0 to 3, default 1).
aligners-cmd-manual-renumber-doc = Changes the FDI number of a tooth; when the new number is already used, the two teeth swap their numbers. The tooth marks exchange their numbers, the tooth colours of the arch follow, the renumbered teeth are measured and closed again (their number decides their axes and centre of resistance) and what the setup and the aligner sequence had for them is dropped. Both numbers must be in the same arch. Parameters: src (required), dst (required).
aligners-cmd-manual-missing-doc = Marks a tooth as missing (agenesis, extraction): its tooth mark, colours, axes, solid and movements are removed, its space stays empty, and the number is kept in the list of missing teeth, which the tooth marks and the brush respect. With missing false the tooth is present again (to be marked or painted, then rebuilt). Parameters: fdi (required), missing (default true).
aligners-cmd-manual-seeds-doc = Adds, moves or deletes the tooth mark of a tooth. A tooth mark is the centre of the occlusal face of a tooth with its number; separating teeth and gum starts from the tooth marks, so the tooth colours of the arch, and what was built from them, are out of date after an edit: they are kept, as in the plugin, the report lists them in invalidated, and separating teeth and gum must be run again. A scan whose arch is still provisional is refused: confirm it first. Parameters: op (add, move or delete), fdi (required), position (case coordinates in mm, required to add or move), snap_to_scan (put the tooth mark on the nearest point of the scan).
aligners-cmd-manual-delete-seed-doc = Removes the tooth mark of one tooth, for example a tooth found by mistake on top of another. The tooth is not recorded as missing (that is Missing tooth): its number can be marked again. As after any change of the tooth marks, the tooth colours of the arch, and what was built from them, are kept but out of date: the report lists them in invalidated, and separating teeth and gum must be run again. A tooth without a tooth mark is refused and nothing changes; a scan whose arch is still provisional is refused too. Parameters: fdi (required).
aligners-cmd-manual-rebuild-tooth = Rebuild one tooth by hand
aligners-cmd-manual-rebuild-tooth-doc = Builds a single tooth again from its region on the scan: from the current tooth colours, from a selection of points that becomes the tooth (its other points go to the gingiva), or after finding the region again from the tooth mark (`resegment`). The region is checked first (size, height, border of the scan, holes) and an invalid one is refused with the reason, changing nothing. The teeth of the arch are then measured again and the closed solid of the tooth replaces the old one; the other teeth keep their solids, axes and movements, and the setup and the aligner sequence of this tooth are out of date. Parameters: fdi (required), vertices, resegment, neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-manual-adjust-doc = Moves and turns one tooth by hand, in the final position (setup) or at one aligner (stage). The movement is an increment on the axes of the tooth, about its centre of resistance: mesial_mm, buccal_mm, extrusion_mm, torque_deg, tip_deg, rotation_deg, and/or a pose delta (t in mm, q as a quaternion x, y, z, w; absolute makes it the new pose; reset brings the tooth back to rest first). With exact the six components are the whole movement from the initial position, the values the inspector shows, not an increment. The result is stored as the other stages store it. The crown is then checked against its neighbours with the collision measure of the aligner sequence: the penetration with each neighbour is reported before and after, and refuse_on_collision turns a collision into an error. A move of the setup leaves the aligner sequence out of date; lock true keeps the tooth where it is put when the setup is computed again, lock false frees it. Parameters: fdi (required), stage, the six components, delta, absolute, exact, reset, lock, refuse_on_collision.
aligners-cmd-manual-tooth-region = Show the region of a tooth
aligners-cmd-manual-tooth-region-doc = Tells from which part of the scan a tooth is built, without changing the case: the points and faces of the scan that carry the tooth colour, the check of that region (size, height, border of the scan, holes; null with region_error when the arch cannot be measured yet) and the margin line, the crown and gingiva boundary the closed tooth is cut along. The margin line is worked out from the tooth colours each time, so it is always the line the next rebuild of the tooth would use: margin gives its points, segments, length_mm, closed (every loop is closed, as the outline of a crown is) and the loops. Parameters: fdi (required) and detail (also lists vertex_indices, face_indices and the points of every polyline in the scan coordinates). An arch that is not separated into teeth yet is refused. The window shows the same data with Show the region in 3D teeth: the tooth in its colour and the other colours dimmed, the margin line in yellow.

## ---- commands: study_base ----
aligners-menu-output-study-models = Study models
aligners-desc-output-study-models = Gives you the orthodontic study models on a trimmed base, in the European or the American (ABO) style.
aligners-cmd-study-models-doc = Builds the orthodontic study models of the arches that have a scan and tooth colours, in occlusion (after the bite registration or the virtual bite), on a trimmed base. Common to both styles: base planes parallel to the occlusal plane, a back plane perpendicular to the midpalatal raphe (estimated from the symmetry of the upper arch) shared by both arches, vertical walls, and the anatomy (teeth, gingiva down to the sulcus, palate) never cut: the base is lowered and the walls pushed out instead, with a warning. ABO: 70 mm for the articulated pair, one third art portion, upper heptagon with heels at 115 degrees, sides at 65 degrees and the front cuts meeting on the raphe, lower front arc from canine to canine. European: walls at 90 degrees, 10 mm base under the anatomy, upper tip on the raphe, lower front flat. The parameters start from the case settings; every field given overrides them. The models built are kept in the case (they replace the earlier ones of their arch, one undoable change) and shown in the 3D view by the "Study models" layer. With dir it also writes study-model-upper.stl and study-model-lower.stl there. Returns the reference, the planes, the outline, the angles and heights designed and measured on the models, the warnings and the arches kept. Parameters: arch, dir, style, total_height_mm, art_fraction, art_thickness_mm, base_thickness_mm, m_base_mm, back_margin_mm, side_angle_deg, lower_side_angle_deg, heel_angle_deg, heel_length_mm, front_angle_deg, side_clear_mm, canine_clear_mm, labial_clear_mm, vestibule_clear_mm, eu_side_angle_deg, eu_heels, sulcus_reach_mm, voxel_mm, tolerance_mm.
aligners-cmd-study-models-settings = Study model settings
aligners-cmd-study-models-settings-doc = Changes the study model settings of the case: the fields given replace the current ones and become the defaults of the study models. Parameters: style (european or abo), total_height_mm, art_fraction, base_thickness_mm, m_base_mm, back_margin_mm, side_angle_deg, heel_angle_deg, side_clear_mm, vestibule_clear_mm.
aligners-study-settings = Base settings
aligners-study-style = Style
aligners-study-style-european = European
aligners-study-style-abo = American (ABO)
aligners-study-total-height = Pair height, ABO (mm)
aligners-study-art-fraction = Art portion, ABO (fraction)
aligners-study-base-thickness = Base thickness, European (mm)
aligners-study-m-base = Clearance under the anatomy (mm)
aligners-study-back-margin = Back margin (mm)
aligners-study-side-angle = Side cut angle, ABO (deg)
aligners-study-heel-angle = Heel angle (deg)
aligners-study-side-clear = Side clearance, ABO (mm)
aligners-study-vestibule-clear = Vestibule clearance, European (mm)
aligners-study-apply = Apply
aligners-study-reset = Revert
aligners-study-no-case = Open a case with the Aligners data to edit the settings.

# Scan import and arch step (commands/import.rs).
# Referenced by the messages below: reads their $role (upper, lower, bite).
aligners-role = { $role ->
    [upper] upper arch
    [lower] lower arch
   *[bite] bite
}

aligners-cmd-import-scans-doc = Imports one or more scans (STL, OBJ, PLY) into the case as one undoable change, like case.import, and recognises the arch from the geometry when a file has neither a role nor a name that tells it (scanner orientation, palate, curve of Wilson, tooth widths). An explicit role wins over the name, the name over the geometry; only a role or the name makes a bite. A scan with fewer than 1000 faces, or one that is not recognised, takes a free arch as provisional. A file with neither a role nor a name that tells it never displaces a confirmed arch (explicit, from the file name or detected): only a scan still provisional gives way to it, by moving to the free arch; otherwise the file takes the free arch as provisional, with an arch conflict warning, or fails with needs_role when no arch is free. A recognition from a weak cue (the palate against the molar cusps, the cusps of a lower arch over a closed model base, or the shape and widths of the teeth alone) gives role_source detected_unsure with a to-be-checked warning: the arch is settled, and case.set-role with the same role on both sides confirms it; any other cue gives detected. A recognition that cannot compare the tooth widths for either arch counts as not recognised. Replacing an arch clears its results, and reports bite_registered in module_cleared when a bite registration had moved the other arch: that registration is stale and has to be done again. Parameters: path, or files with an optional role each; role is auto, upper, lower or bite. The output has the fields of case.import plus detections: arch, source and confidence of each recognised file.
aligners-cmd-detect-arch = Arch recognition
aligners-cmd-detect-arch-doc = The arch step of the pipeline. It fails if an arch scan has fewer than 1000 faces. It recognises from the geometry the arch of the scans still provisional and puts them on it, swapping two provisional scans if needed; moving a scan clears its results. When two scans claim one arch the surer one takes it and the other stays provisional on the opposite arch, with a warning. Explicit, file name, detected and detected_unsure roles are left alone and reported unchanged. A recognition from a weak cue is stored as detected_unsure, with a to-be-checked warning. No parameters. The output lists every scan with its outcome (unchanged, resolved, conflict, not_recognised), from, to, role_source and detection, plus moved, warnings, world_frame_reset and module_cleared.

aligners-import-error-path-or-files = Give exactly one of path (one file) or files (a list of files).
aligners-import-error-no-files = The list of files is empty: give at least one file.
aligners-import-error-nothing = No scan was imported. { $reasons }
aligners-import-closing = Closing the scans with a base

aligners-arch-error-no-scan = The case has no upper or lower scan: import one first.
aligners-arch-error-not-dental = The selected file does not look like a dental scan: { $name } has only { $faces } faces.
aligners-arch-error-provisional = The arch of { $name } is not confirmed yet: it is only a guess ({ aligners-role }). Open the Import scans panel and press Confirm next to the scan, or choose the right arch.

aligners-arch-warn-not-recognised = { $name }: the arch could not be recognised from the geometry. Set the role by hand.
aligners-arch-warn-to-be-checked = { $name }: recognised as { aligners-role }, but the cue is weak (palate, molar cusps or tooth widths disagree or are not enough). Check it once teeth and gum are separated, then press Confirm next to the scan in the Import scans panel.

# Corrections of a scan (commands/set_arch.rs, commands/make_base.rs).
aligners-cmd-set-arch = Choose the arch
aligners-cmd-set-arch-doc = Says by hand which arch a scan is, and turns its occlusal side over if needed, as one undoable change. Parameters: scan (upper or lower: the arch the scan has now; default the only arch scan), arch (upper or lower: the arch it is; default the one it has; a scan already there takes the other arch), flip_occlusal (true to turn the occlusal side over, again to turn it back). The role becomes explicit. With a new arch the tooth marks, tooth colours, teeth marked missing and control visits of the moved arches are renumbered to the other arch, same side and same tooth (13 becomes 43, 24 becomes 34), the patient is no longer placed on the axes, and 3D teeth already built are built again, which clears the final position and the aligners of the arch and makes the diagnosis again. Turning the occlusal side over turns the arch over in every step and clears nothing: the separation and what follows it are out of date. The output gives scan_id, source_name, from, to, role_source, swapped, renumbered, rebuilt (teeth built again), flipped, occlusal_flipped, world_frame_reset, module_cleared, invalidated and teeth_build.
aligners-cmd-make-base = Close with a base
aligners-cmd-make-base-doc = Closes an open arch scan with a flat base, as one undoable change. Parameters: arch (upper or lower; default the only arch scan), height_mm (thickness of the base under the lowest point of the scan, 0.5 to 20 mm, default 3). The base is perpendicular to the occlusal direction of the arch frame of the tooth marks, so the arch needs its marks on both sides and on the incisors. Every open border of the scan is extruded down to the base plane and closed flat; the points of the scan keep their place and number, and the new points take the tooth colour of the point they come from, so nothing computed is cleared. A scan that is already closed is left as it is (already_closed). Not needed for printing: the export closes the models by itself. The output gives arch, scan_id, already_closed, closed_edges, vertices_added, faces_added, height_mm, watertight and labels_extended.
aligners-err-base-height = The base height must be between 0.5 and 20 mm (got { $value }).

# The ids of module_cleared (dca_model::CLEARED_IDS), underscores as hyphens.
aligners-cleared-seeds = tooth marks
aligners-cleared-missing = teeth marked missing
aligners-cleared-labels = teeth and gum
aligners-cleared-tooth-frames = tooth axes
aligners-cleared-tooth-geometry = tooth measures
aligners-cleared-crowns = 3D teeth
aligners-cleared-gingiva = gingiva
aligners-cleared-diagnosis = diagnosis
aligners-cleared-bolton = Bolton ratios
aligners-cleared-complexity = complexity score
aligners-cleared-setup = setup
aligners-cleared-stages = aligner sequence
aligners-cleared-review = clinical review
aligners-cleared-ramps = bite ramps
aligners-cleared-visits = control visits
aligners-cleared-study-models = study models
aligners-cleared-bite-registered = bite registration (to be redone: the other arch stays where the registration moved it)

# Errors a command reports when a step it depends on has not been done
aligners-err-arch-upper = upper
aligners-err-arch-lower = lower
aligners-err-no-data = the Aligners data cannot be read
aligners-err-no-case-data = the case has no Aligners data
aligners-err-arch-not-segmented = the { $arch } arch has no tooth colours: run { aligners-menu-preparation-segmentation-automatic } first
aligners-err-none-segmented = no arch has tooth colours: run { aligners-menu-preparation-segmentation-automatic } first
aligners-err-labels-missing = the tooth colours of the { $arch } arch are missing
aligners-err-tooth-labels-missing = the tooth colours are missing
aligners-err-labels-mismatch = the tooth colours of the { $arch } arch do not match the scan ({ $labels } colours, { $vertices } points)
aligners-err-tooth-labels-mismatch = the tooth colours do not match the scan ({ $labels } colours, { $vertices } points)
aligners-err-labels-stale = the tooth colours of the { $arch } arch do not match the scan
aligners-err-no-scan = the case has no { $arch } scan
aligners-err-no-scan-any = the case has no upper or lower scan
aligners-err-scan-missing = the { $arch } scan is missing
aligners-err-reference-missing = the reference scan is missing
aligners-err-arch-no-seeds = the { $arch } arch has no tooth marks: run { aligners-menu-teeth-find } first
aligners-err-none-seeded = no arch has a scan with tooth marks: run { aligners-menu-teeth-find } first
aligners-err-stopped-at = Stopped at { $step }: { $reason }
aligners-err-seed-degenerate = the scan of the { $arch } arch is empty or damaged
aligners-err-seed-arch-not-found = no dental arch found in the { $arch } scan: place the tooth marks by hand with { aligners-menu-preparation-numbering-place-seeds }, in { aligners-menu-teeth-find }
aligners-err-seed-too-few = only { $found } teeth found on the { $arch } arch: place the tooth marks by hand with { aligners-menu-preparation-numbering-place-seeds }, in { aligners-menu-teeth-find }
aligners-err-seed-bad-ends = the last tooth of each side must be number 5, 6, 7 or 8
aligners-err-seed-bad-tooth = { $fdi } is not a valid tooth number
aligners-err-deciduous = { $fdi } is a deciduous (milk) tooth: only permanent teeth (11 to 48) are supported
aligners-err-tooth-not-closed = tooth { $fdi } of the { $arch } arch could not be closed into a solid: every try leaves a hole or an edge shared by more than two faces; check its crown with the brush, then build the teeth again
aligners-warn-seeds-close = the marks of teeth { $a } and { $b } are only { $mm } mm apart, less than half the width of a tooth: the separation may leave one of them without a crown; check that each mark sits on its own tooth
aligners-region-summary = Tooth { $fdi }: { $vertices } vertices, { $area } mm², { $width } mm wide
aligners-region-vertices = Tooth { $fdi }: { $vertices } vertices
aligners-region-refused = Tooth { $fdi }: { $issues }
aligners-region-warned = { $summary }. Warning: { $issues }
aligners-region-no-region = tooth { $fdi } has no region on the scan: paint it or give it a selection
aligners-region-too-small = region too small for a crown ({ $area } mm², { $width } mm wide; tooth { $fdi } is about { $typical } mm wide): enlarge it or paint it again
aligners-region-too-low = region too low ({ $height } mm): it looks like gingiva, not a crown
aligners-region-too-wide = region very wide ({ $width } mm, typical tooth { $typical } mm): it may include two teeth or some gingiva
aligners-region-split = the region is split in { $parts } parts: only the largest ({ $share }%) becomes the tooth (clean it or paint it again)
aligners-region-on-border = the region touches the border of the scan along { $share }% of its outline: the crown is not surrounded by gingiva (incomplete scan or wrong region)
aligners-region-touches-border = the region touches the border of the scan ({ $vertices } vertices): the missing part will be rebuilt
aligners-region-big-hole = the region has a hole as big as the crown (outline { $outline } mm): it is not a crown; fill it with the brush or clean the tooth colours
aligners-region-holes = the region has { $count } holes (gingiva inside the crown, outline up to { $outline } mm), closed without anatomical shape: better fill them with the brush or clean the tooth colours
aligners-err-frame-too-few-seeds = the { $arch } arch has fewer than 4 tooth marks: at least 4 (incisors and molars) are needed to place the arch; let the program find them with { aligners-menu-teeth-find }, or place them by hand with its tool { aligners-menu-preparation-numbering-place-seeds }
aligners-err-frame-one-side = the { $arch } arch needs tooth marks on both sides and on the incisors to place the arch; let the program find them with { aligners-menu-teeth-find }, or place them by hand with its tool { aligners-menu-preparation-numbering-place-seeds }
aligners-err-frame-degenerate = the scan of the { $arch } arch is empty or damaged
aligners-err-seg-part-scan = scan
aligners-err-seg-part-simplified = simplified scan
aligners-err-seg-fault-no-triangles = it has no triangles
aligners-err-seg-fault-index = a triangle uses a point that does not exist
aligners-err-seg-fault-not-finite = a point has an invalid position
aligners-err-seg-bad-mesh = the { $part } of the { $arch } arch cannot be used: { $fault }
aligners-err-seg-seed-not-finite = a tooth mark of the { $arch } arch has an invalid position
aligners-err-seg-decimate = the scan of the { $arch } arch could not be simplified: { $detail }
aligners-err-seg-transfer = the tooth colours of the { $arch } arch could not be carried to the full scan: { $detail }
aligners-err-seg-target-faces = target_faces must be a number between 5000 and 200000 (got { $value })
aligners-err-seg-eta-convex = eta_convex must be a number between 0.01 and 1 (got { $value })
aligners-err-seg-sigma = sigma must be a number between 0.1 and 10 (got { $value })
aligners-err-auto-keep-seeds = the { $arch } arch has no tooth marks: mark the teeth, or let the program find them
aligners-err-no-aligners = there are no aligners yet: run { aligners-menu-planning-aligners } first
aligners-err-no-measured-teeth = no arch has measured teeth: run { aligners-menu-teeth-build } first
aligners-err-then-measure = { $reason }: run { aligners-menu-teeth-build } first
aligners-err-no-3d-teeth = the { $arch } arch has no 3D teeth: run { aligners-menu-teeth-build } first
aligners-err-none-3d-teeth = no arch has 3D teeth: run { aligners-menu-teeth-build } first
aligners-err-aligner-split = the { $arch } aligner { $stage } would come apart: the teeth { $teeth } are not joined to the rest of the aligner, not even by a bar over the space between the teeth; check the missing teeth and the tooth marks, then build it again
aligners-err-print-check-failed = the files are in { $dir }, but { $count } of them fail the print check and must not be printed as they are: { $files }
aligners-err-print-no-ids = give at least one aligner, like SUP-03 or INF-12-P
aligners-err-print-unknown-id = { $id } is not an aligner of this case: use the ids of the list of aligners to print, like SUP-03 or INF-12-P
aligners-err-print-transition = { $id } cannot go from “{ $from }” to “{ $to }”
aligners-err-print-printed-by-app = { $id } is marked printed only when you confirm it or the printer reports it done
aligners-err-diagnosis-edit-empty = give at least one value to write or to reset
aligners-err-diagnosis-edit-field = { $field } is not a value of the case analysis: use discrepancy, little, midline_offset or largest_space
aligners-err-diagnosis-edit-range = { $field } must be a number between { $min } and { $max } mm (got { $value })
aligners-err-diagnosis-edit-nothing = the case analysis has not been run: run { aligners-menu-diagnosis-space-analysis } first
aligners-err-diagnosis-edit-first = the { $arch } arch has no case analysis: run { aligners-menu-diagnosis-space-analysis } first
aligners-err-diagnosis-edit-arch = both arches have a case analysis: give the arch (upper or lower)
aligners-err-both-arches = the { $arch } arch has no tooth colours: this step needs both arches, with teeth and gum separated
aligners-err-arch-why = the { $arch } arch: { $why }
aligners-err-why-not-separated = teeth and gum are not separated: run { aligners-menu-preparation-segmentation-automatic } first
aligners-err-why-not-measured = the teeth are not measured: run { aligners-menu-teeth-build } first
aligners-err-why-no-setup = there is no final position: run { aligners-menu-planning-setup-automatic } first
aligners-err-setup-bite-first = with both arches the bite comes first (the relation between the arches): put the arches in occlusion with { aligners-menu-occlusion-align }, then compute the final position
aligners-err-setup-line-arch = both arches have tooth colours: give the arch of the line (upper or lower)
aligners-err-setup-line-points = the arch line needs at least two points, each with three finite coordinates
aligners-err-setup-settings-range = { $name } must be between { $min } and { $max } (got { $value })
aligners-err-staging-settings-range = { $name } must be between { $min } and { $max } (got { $value })
aligners-err-production-settings-range = { $name } must be between { $min } and { $max } (got { $value })
aligners-err-production-stamp-code = the case code { $code } cannot be stamped: at most 8 letters, digits and -
aligners-err-aligner-end-fdi = give the last tooth the aligner covers (fdi)
aligners-err-aligner-end-tooth = { $fdi } is not a tooth of the case: the aligner can end only on a tooth of its arch
aligners-warn-aligner-end-setup = The final position and the aligners were computed with another end of the aligner. Compute the final position again (the teeth left out stay where they are), then the aligners.
aligners-err-start-date = the start date { $value } is not a date: write it as YYYY-MM-DD, or leave it empty for the day the calendar is made
aligners-err-tooth-speed-range = the speed of a tooth goes from { $min } to { $max } (got { $value })
aligners-err-tooth-speed-no-tooth = tooth { $fdi } is not one of the 3D teeth of the case: build the teeth first with { aligners-menu-teeth-build }
aligners-err-bite-no-penetration = the arches do not interpenetrate: nothing to correct
aligners-err-nothing-to-stage = no arch is ready: run { aligners-menu-preparation-segmentation-automatic }, { aligners-menu-teeth-build } and { aligners-menu-planning-setup-automatic } first
aligners-err-no-teeth-any = the teeth are not measured: run { aligners-menu-preparation-segmentation-automatic } and { aligners-menu-teeth-build } first
aligners-err-arch-no-teeth = the { $arch } arch has no measured teeth: run { aligners-menu-teeth-build } first
aligners-err-arch-no-plan = the { $arch } arch has no measured teeth or no final position: run { aligners-menu-teeth-build } and { aligners-menu-planning-setup-automatic } first
aligners-err-aligner-not-planned = aligner { $aligner } is not in the plan ({ $total } aligners): run { aligners-menu-planning-aligners } first, or choose an earlier aligner
aligners-err-review-no-diagnosis = the { $arch } arch has no case analysis: run { aligners-menu-diagnosis-space-analysis } first
aligners-err-review-nothing = no arch has a case analysis to classify: run { aligners-menu-diagnosis-space-analysis } first
aligners-err-review-none = there is no clinical review yet: run { aligners-menu-review-classify } first
aligners-err-review-key = there is no review item { $key }
aligners-err-review-value = { $value } is not one of the values of this item ({ $options })
aligners-err-no-visit = there is no control visit to start from: run { aligners-menu-followup-visits } first
aligners-err-visit-arch = could not tell whether this scan is the upper or the lower arch: put upper or lower in the name of the file and try again
aligners-err-tooth-not-separated = tooth { $fdi } has no colour on the scan
aligners-err-tooth-not-measured = tooth { $fdi } is not measured: run { aligners-menu-teeth-build } first
aligners-err-tooth-unknown = tooth { $fdi } does not exist: it has no tooth mark, colour or measures
aligners-err-tooth-no-mark = tooth { $fdi } has no tooth mark
aligners-err-tooth-has-mark = tooth { $fdi } already has a tooth mark: move it, or delete it first
aligners-err-seq-no-steps = give at least one step
aligners-err-seq-too-many = { $steps } steps for { $teeth } teeth left to number in the { $arch } arch
aligners-err-seq-wrong-tooth = step { $step } is for tooth { $fdi }, but the next tooth to number is { $expected }: the tooth marks changed since the numbering started, start it again
aligners-err-no-crown-at-mark = no crown found around the tooth mark of tooth { $fdi }: move the mark or paint the tooth
aligners-err-scale-tooth-only = only a tooth can be scaled: the gingiva and the scans can be moved and turned
aligners-err-tooth-not-built = tooth { $fdi } has no 3D tooth: run { aligners-menu-teeth-build } first
aligners-err-tooth-scale-range = the size of tooth { $fdi } must stay between { $min } and { $max } times the size it was built with

aligners-cmd-numbering-sequence = Number the teeth in order
aligners-cmd-numbering-sequence-doc = Numbers the teeth of an arch one after the other, from the patient's right distal tooth to the left distal tooth (upper: 18 to 11, then 21 to 28; lower: 48 to 41, then 31 to 38). Each step is for the next tooth and may name it (fdi): a step named for another tooth refuses the whole numbering; a point on the tooth puts its tooth mark and gives it its FDI number; a skip step leaves that tooth without a mark and goes on to the following one, as the S key does in the plugin (the tooth is not added to the missing teeth: for that use Missing tooth). The third molars are left out unless asked, and so are the teeth that already have a tooth mark or are missing. Fewer steps than teeth is fine (Enter or Esc in the app): the tooth marks given are kept and the report names the tooth that comes next. All the steps are one change that can be undone at once. What was computed from the tooth marks of the arch (tooth colours, axes, solids, setup, aligner steps) is kept, as in the plugin, but is out of date: the report lists it in invalidated, and those steps must be run again. A scan whose arch is still provisional is refused: confirm it first. Parameters: steps (required; each step has a kind, either point with a position [x, y, z] in case coordinates in mm, or skip, and an optional fdi), arch (upper or lower; default: the only scan of the case), third_molars (default false), snap_to_scan (put each tooth mark on the nearest point of the scan).
aligners-cmd-numbering-clear-seeds = Clear the tooth marks
aligners-cmd-numbering-clear-seeds-doc = Removes all the tooth marks of the case so that the numbering starts again from zero; the list of missing teeth stays, as in the plugin, unless keep_missing is false. What was computed from the tooth marks (tooth colours, axes, solids, setup, aligner steps of the arch) is kept, as in the plugin, but is out of date; the report lists it in invalidated. Parameters: arch (upper or lower; default: both arches), keep_missing (keep the list of missing teeth; default true).
aligners-cmd-find-teeth-settings = Settings of Find the teeth
aligners-cmd-find-teeth-settings-doc = Changes the settings of Find the teeth in the case: third_molars (look for the third molars too, default false) and missing (FDI numbers of the teeth known to be missing; replaces the list; 11 to 18, 21 to 28, 31 to 38, 41 to 48). A field not given stays as it is. They are the values Find the teeth uses when it is run from its inspector, like the plugin's Search also the third molars and Missing teeth fields. A number that is not a permanent tooth is refused and nothing changes.
aligners-find-third = Search also the third molars
aligners-find-missing = Missing teeth
aligners-find-missing-hint = e.g. 35, 45
aligners-find-missing-detail = FDI numbers separated by commas. They are applied when you leave the field.
aligners-find-missing-error = Not a tooth number: { $list }. Use FDI numbers: 11 to 18, 21 to 28, 31 to 38, 41 to 48.
aligners-find-found = Teeth found
aligners-find-upper = Upper arch
aligners-find-lower = Lower arch
aligners-find-no-scan = Import the scan of an arch to see its teeth.
aligners-find-count = { $n } teeth numbered
aligners-find-legend = Filled box: the tooth has a mark. Crossed box: the tooth is missing.
aligners-find-cell-marked = { $fdi }: has a mark
aligners-find-cell-missing = { $fdi }: missing
aligners-find-cell-empty = { $fdi }: no mark

aligners-cmd-teeth-build-full-doc = Gives every tooth as a closed 3D model, builds the gum that follows the teeth and makes the diagnosis, as one undoable change, as the plugin does. An arch is built in one of two ways. All its teeth: Close all the 3D teeth, which measures the teeth first, then Gingiva; the final position and the aligners of the arch are cleared, since they were planned on the old teeth. Some teeth: the teeth given in teeth or, without it, the teeth whose colours were corrected by hand since the teeth were built (with the teeth coloured but not built and the built teeth left without colour); the teeth of the arch are measured again, each of those teeth is closed again (a tooth with no colour left is removed, as a missing tooth), then Gingiva; the other teeth keep their solids, a rebuilt tooth keeps its planned movement and its lock, the aligners of the arch are cleared. Without teeth, an arch already built with teeth corrected by hand is built the second way, otherwise the first. Then Case analysis runs on the new teeth. Needs teeth and gum separated. Returns every step with its output, arches (for each arch: mode all or some, teeth, rebuilt, gingiva), removed, setup_cleared, stages_cleared and diagnosis. Parameters: arch (all, upper or lower), teeth (FDI numbers of the teeth to build again).
aligners-err-teeth-build-not-tooth = { $fdi } is not a tooth number: use FDI numbers, 11 to 18, 21 to 28, 31 to 38, 41 to 48
aligners-err-teeth-build-other-arch = tooth { $fdi } is in the { $arch } arch, which was not asked for
aligners-cmd-segmentation-check = Check the separated teeth
aligners-cmd-segmentation-check-doc = Checks that the teeth found by the separation are plausible, as the plugin's automatic analysis does: boundaries between two teeth with no groove between them, pieces narrower than a crown, crowns wider than one tooth. With correct the tooth marks are corrected (two parts of one crown get one mark, a piece or a mark in a gap loses its mark, a crown of two teeth gets two marks) and the arch is separated again, at most three times; after a correction the teeth are numbered again from their widths along the arch and separated with the new numbers, unless some teeth are still not plausible. Without correct (marks placed by hand) the problems are only reported. Find everything automatically runs it after the separation, correcting the arches whose marks it proposed. An arch where no arch curve is found is not checked and is listed in skipped. Returns for each arch rounds, actions, renumbered, corrections, problems, issues, notes and spans. Parameters: arch (all, upper or lower), correct (default false), third_molars, missing.
aligners-check-act-drop = { $fdi } was not a whole tooth: its mark was removed
aligners-check-act-merge = { $keep } and { $drop } were parts of the same tooth: joined as { $keep }
aligners-check-act-split = { $fdi } held two teeth: split in two
aligners-check-act-renumbered = teeth numbered again from their widths: { $list }
aligners-check-issue-no-fold = { $a } and { $b } have no groove between them (a boundary of { $mm } mm over the crown)
aligners-check-issue-narrow = { $fdi } is only { $mm } mm wide along the arch
aligners-check-issue-wide = { $fdi } is { $mm } mm wide along the arch (more than one tooth)
aligners-check-note-not-renumbered = the teeth were not numbered again from their widths: some teeth are still not plausible
aligners-check-note-one-premolar = only one premolar in quadrant { $quadrant } ({ $fdi }): the widths do not tell the first from the second premolar
aligners-check-note-uncertain = numbering uncertain from the widths alone ({ $pairs })
aligners-check-or = { $a } or { $b }
aligners-auto-warn-ratio = Incisor / molar ratio { $ratio } unusual for the { $arch } arch: check the arch
aligners-auto-warn-virtual-bite = Arches put in VIRTUAL occlusion before the final position: for the clinical relation, register the vestibular bite
aligners-err-teeth-build-empty = no tooth to build again: give at least one FDI number in teeth, or leave it out to build the teeth that changed

## simple mode
aligners-find-summary = Teeth found: { $n }. Missing: { $missing }.

# Print tracking (print_ui.rs)
aligners-print-no-aligners = There are no aligners yet: make them with { aligners-menu-planning-aligners }.
aligners-print-count = Printed { $printed } of { $total }
aligners-print-missing = Still to print: { $count }
aligners-print-no-export = No files yet: export the aligners with { aligners-menu-output-export-all }.
aligners-print-status-missing = To print
aligners-print-status-queued = Queued
aligners-print-status-sent = Sent
aligners-print-status-printed = Printed
aligners-print-status-failed = Failed
aligners-print-status-reprint = To print again
aligners-print-mark = Mark…
aligners-print-mark-printed = Printed
aligners-print-mark-failed = Failed
aligners-print-mark-reprint = To print again
