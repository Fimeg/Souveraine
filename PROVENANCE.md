# Provenance

This branch is a projection, not a development branch. Its history begins at a
single root commit, and the tree under that commit was constructed from an
internal source under a manifest that decides which paths are allowed to leave.
Development is canonical elsewhere and is not rewritten to produce this.

Every publication carries three trailers:

    Source-Sha    the internal commit the tree was constructed from
    Policy-Sha    the manifest in force when it was constructed
    Tree-Digest   a digest of the published tree

They bind the projection to what produced it without pretending the public
commit is the private one: same lineage, different tree, and the record says so.

## Reproducing Tree-Digest

`Tree-Digest` is the trailer you can check, and it is here to be checked. Clone
this branch and run:

    LC_ALL=C git ls-tree -r --full-tree HEAD^{tree} | LC_ALL=C sort | sha256sum

The result must equal the `Tree-Digest` trailer on the commit you are verifying.

`LC_ALL=C` is not decoration. Entries that share a blob are ordered by path, and
without a pinned collation that order follows whatever locale the verifier
happens to be running -- so two machines holding a byte-identical tree compute
different digests and each concludes the other is lying. The first epoch of this
branch recorded a digest produced under an unpinned locale. It was reproducible
on exactly one machine, which is the same as not being reproducible at all.
Pinning the collation is the whole fix, and it is why this file exists.

## What you cannot check, said plainly

`Source-Sha` and `Policy-Sha` name objects in a private forge. They are recorded
so that whoever can reach that forge can audit the construction end to end, and
they are of no use to anyone who cannot. Stating that is more honest than
implying a verification that is not on offer.

## What this branch does not carry

The construction machinery -- the manifest and the gate that enforces it -- is
not published. A gate that names the strings it refuses publishes those strings,
which is precisely the disclosure it exists to prevent. That is not theoretical:
an earlier epoch of this branch shipped the gate under a reviewed exception
reasoning that a gate necessarily carries the patterns it hunts for. True, and
still wrong, because the exception granted itself permission at the one boundary
it was built to hold.

So the rules run against this tree before it leaves, and what leaves is the tree
and the trailers above -- not a copy of the rules.

## Force pushes

Publication is fast-forward only. A non-fast-forward update to this branch means
a deliberate, recorded decision was made, not that something routine happened.
