<!-- SPDX-License-Identifier: MIT -->

# Interrupted CAJ PDF objects

Tracking: #226. This work does not yet complete conversion of the six failed
baseline sources. Source files and diagnostic mutations remain outside Git.

## Measured prefixes

All three discovery inputs match the SHA-256 values in the conformance matrix.
Offsets below are absolute in the original source. Prefix comparison excludes
only trailing ASCII whitespace; the remaining bytes match exactly.

| Input | Partial object starts | Following header candidate | Matching same-reference header | Matching bytes |
| --- | ---: | ---: | ---: | ---: |
| issue-25 | 116653 | 116724 | 457263 (later) | 69 |
| issue-92 | 19103 | 19116 | 14482 (earlier) | 11 |
| issue-30 | 189169 | 189264 | 191653 (later) | 93 |

The first two failures interrupt dictionaries. The third interrupts a Flate
payload. Candidate locations alone are not sufficient to accept a repair.
An external experiment removing only those prefixes and adjusting the CAJ
page table encounters additional malformed objects in all three files;
there is no successful conversion result from that experiment.

## Admitted known-dictionary rule

On an `expected PDF name` error within 256 bytes of the current object start,
find exactly one already indexed object with the same reference. It must be
a complete non-stream dictionary. Compare its bytes with at most 256 bytes
at the current object start. The exact common prefix, followed only by ASCII
whitespace, defines one candidate boundary. The next byte must be a digit;
normal scanning must then parse an indirect object at that exact boundary.
All subsequent object/link/stream validation still applies before output.

This handles a cut between the two closing dictionary brackets or inside a
key. It does not search for object markers or stream terminators. Changed
fields that do not leave a valid next object fail; unknown references,
conflicting complete objects, stream dictionaries, incomplete literals and
invalid next-object syntax retain errors. No objects are synthesized: the
complete previously indexed dictionary remains in the reconstruction plan.
The existing input, metadata, cancellation and output budgets remain active.

Original tests use invented nested dictionaries and a different following
object, including cuts after an opening dictionary, inside a key and between
closing brackets. Negative controls cover altered data, unknown/overflowing
object IDs, streams, literals and non-object suffixes.

## Current real-source result

The original issue-92 source passes its interrupted dictionary prefixes with
this rule, then stops at byte 288441, object 186, because stream Length has no
unique bounded repair. No final PDF is published. Issue-25 and issue-30 retain
their initial errors because their matching complete objects occur later;
this rule deliberately requires an already validated dictionary.

Remaining work in #226 includes full-source recovery where uniquely justified,
all six baseline classifications, independent page/content validation and the
three public interfaces. No recovered-complete-document claim is made here.
External receipts are in `caj2pdf-caj-failure-diagnostics-20261001`.
