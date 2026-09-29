<!-- SPDX-License-Identifier: MIT -->

# T.88 MQ state-data provenance decision

Current project decision: [2026-09-29 standard numeric state adoption](provenance.md#2026-09-29-standard-numeric-state-adoption-3044).
The owner has directed direct use without emailing rights holders. The following
research record remains historical and is not a current implementation blocker.

Status: **UNRESOLVED**. Reviewed on 2026-09-27 by Codex repository research and an independent Codex reviewer for [issue #44](https://github.com/rwv/caj2pdf-rust/issues/44). This is a repository release decision, not a determination that the numeric data is or is not copyrightable. The exact state rows remain outside Git and all native, WASM, and JavaScript packages. A new independent review is required before relying on a different decision.

## Material and editions

The target is the **47-state** probability-transition data in Annex E.2.5, Table E.1 of [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), approved 10 February 2000. The consulted official English PDF has SHA-256 `a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`; its published copy bears an ITU 2002 copyright notice. Its text is also published as ISO/IEC 14492:2001. The [ITU edition index](https://www.itu.int/rec/T-REC-T.88/en) lists later 2003 amendments and a 2018 edition. This review concerns the **unamended 2000 base edition** used by the repository's private conformance fixture. No later amendment or 2018 software contribution is assumed to change the rights in its Table E.1.

Four distinct things must stay distinct:

1. The normative arithmetic-decoder behavior and the ability to write independent Rust control flow.
2. The published expression and exact values of the 47-state transition table.
3. The original MIT `jbig2::mq` implementation, which accepts caller-supplied state rows and does not contain the published rows.
4. The official Annex H.2 bytes/checkpoints and external HN/C8 corpus, held only as private conformance inputs.

Passing Annex H.2 and the private generic-only and text-only pixel-hash comparisons proves behavior only on those tested slices. It does not grant rights to redistribute the published state data or establish general compatibility for every valid stream. The observed HN/C8 modes and reachable states are corpus observations, not format-wide guarantees.

## Sources and scope of permission

The [T.88 publication](https://www.itu.int/rec/T-REC-T.88-200002-S/en) is freely accessible, but its front matter reserves reproduction and use of the publication without written ITU permission. Its patent notice discusses claimed patents; it is not a copyright license for the numeric table. [ISO's copyright page](https://www.iso.org/copyright.html) also requires permission to reproduce its publications, and [ITU's copyright page](https://www.itu.int/en/Pages/copyright.aspx) warns that third-party rights may apply. The joint ITU/ISO publication and any contributor rights may be relevant. The public material reviewed here does not establish who can grant MIT redistribution rights to the exact Table E.1 data.

The [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) are dated 7 December 2011 and [apply from 13 April 2012](https://www.itu.int/en/itu-t/ipr/pages/default.aspx). The [official revision history](https://www.itu.int/en/ITU-T/ipr/Pages/revsoft.aspx) starts with trial guidelines in June 2002, after the base T.88 approval. The public sources do not establish that these rules apply retroactively to the 2000 table. Section 2.2.2 discusses software describing data structures, data streams, and schemas, and says implementers can use that material in implementations without copyright assertions. It does not identify Table E.1 as that category or expressly authorize labeling a copy of its exact numeric rows as MIT in source, generated files, or downstream packages. Annex B permits evaluation and conformance testing for limited purposes; that is distinct from unrestricted MIT redistribution.

The [ITU software-declaration database filtered to T.88](https://www.itu.int/net4/ipr/search.aspx?sector=ITU&class=SW&rec=T.88&prod=T.88&opt=-1&field=abcjn) returned one record on 2026-09-27: [T88_S01](https://www.itu.int/net4/ipr/details_sw.aspx?sector=ITU-T&id=T88_S01), registered 12 October 2018. It identifies **ICT Link**, software name **“Sample software”**, and Option 1.3 on the 7 December 2011 form. Its signed [archived declaration](https://www.itu.int/dms_pub/itu-t/oth/04/07/T040700093A0001PDFE.pdf) (SHA-256 `69afb7b76ae2cd762eaf2c59606438064c9b41f3daf98c73eab4bd692041ab50`, both pages visually checked) identifies **T.88 (2000)/Amendment 4** and ISO/IEC 14492:2001/Amendment 4, and is dated 30 September 2018. The [ITU work-programme entry](https://www.itu.int/ITU-T/workprog/wp_item.aspx?isn=9199) describes the Amendment 4 work item as a JBIG2 encoder/decoder verification test procedure and marks that work item **Discontinued**; this does not establish that the later declaration is void. It is distinct from both the 2000 base edition and the 2018 second edition. This later sample-software declaration does not identify the base edition's Table E.1 states or prove that ICT Link controls those states. Option 1.3's [Annex C terms](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) permit derivatives, reproduction, distribution, and sublicensing of the *identified software* only for specified conforming implementation, evaluation, and conformance purposes; they are purpose-limited and do not themselves relicense unrelated Table E.1 data under MIT. [ITU cautions that its declarations database may be incomplete](https://www.itu.int/ITU-T/dbase/copyright/readme_dbase.html), so one listed record is not evidence that no other rights exist.

No reproducible independently authored mathematical state model that yields all exact 47 rows has been established. The current implementation does not derive them: `MqTable::new` validates rows provided by a caller. Copying, transcribing, rearranging, or generating the published rows from another decoder would not satisfy the original-derivation path in #44.

## Release-surface inventory and decision

| Surface | Current state |
| --- | --- |
| `crates/caj2pdf-core/src/jbig2/mq.rs` | Original MIT arithmetic control flow, `MQ_STATE_COUNT = 47`, and a caller-supplied `MqState` type; no normative rows. |
| Tests | Invented state machines in committed tests. The exact Annex H.2 state/vector fixture is SHA-pinned and external; a clean clone does not run it. |
| Examples and diagnostics | Load the private fixture only when explicitly supplied; no fixture generator or exact-state fallback is shipped. |
| Native/WASM/JS packages | Compile the table-supplied core without the exact Table E.1 rows. The JS package does not embed a derived table. |
| External corpus and generated output | CAJSamples documents, official PDF, Annex H bytes, pixel outputs, and exact-state fixture remain outside tracked files and release artifacts. |

**Decision:** path (a), a rights-holder grant or independently reviewed rights basis for MIT source and generated artifacts, is unproven. Path (b), a reproducible genuinely original exact state derivation, is also unproven. The repository therefore remains `UNRESOLVED` for exact-state redistribution. #44 stays open and continues to block #9 and any standalone or integrated JBIG2 pixel decoder that bundles the states or claims shipped standard compatibility. Experimental, caller-table decoding and private diagnostics may continue without bundling the table or treating `NOT_RUN` as a compatibility pass.

## Proposed inquiry — not sent

To: ITU-T IPR contact listed on the [official IPR page](https://www.itu.int/en/itu-t/ipr/pages/default.aspx)

Subject: Rights clarification for ITU-T T.88 (02/2000) Annex E.2.5 Table E.1 numeric MQ states

> We maintain an MIT-licensed Rust project for CAJ/HN/C8 image conversion. We wrote the T.88 arithmetic decoder control flow independently and currently require callers to supply the 47 MQ state rows privately. We seek a written clarification of who controls rights in the exact numeric contents of Table E.1 in the unamended T.88 (02/2000) / ISO/IEC 14492:2001 edition.
>
> May those exact numeric state values be included in MIT-licensed source and generated native, WebAssembly, and JavaScript packages for worldwide commercial and noncommercial use, modification, sublicensing, and downstream redistribution? If permission is available, please identify the authorized rights holder, required notices, the exact edition/table covered, and whether a separate grant from ISO/IEC or any contributor is needed.
>
> We found the T88_S01 “Sample software” Option 1.3 declaration registered in 2018. Does it cover Table E.1 of the 2000 base edition, or only its named sample software? Do the 2011/2012 ITU Software Copyright Guidelines, including §2.2.2, apply to these numeric states from the earlier edition? If ITU cannot answer or grant the needed rights, please direct us to the appropriate rights holder.

The draft is for repository-owner review. It has not been transmitted.
