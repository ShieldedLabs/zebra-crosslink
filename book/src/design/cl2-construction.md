# The Crosslink 2 Construction

```admonish warning "Attention"
This section is an almost direct paste from a section found in `https://github.com/Electric-Coin-Company/tfl-book` for the purposes of precisely committing to a (potentially incomplete, confusing, or inconsistent) precise set of book contents for a security design review.

We believe this respects the license of the other repository. However, the other authors of the TFL-Book, Daira Hopwood and Jack Grigg have not reviewed and do not endorse anything found in this repository in terms of design or otherwise.
```

```admonish warning "Attention"
The `zebra-crosslink` design in this book diverges from this text by _not_ including "Stalled Mode" rules. This simplification to the protocol rules follows the rationale that we are already relying heavily on the stakers to guard safe operation of the protocol by redelegating appropriately to prevent hazards like BFT stalls.
```

```admonish warning "Attention"
The `zebra-crosslink` design does not use the "outer signature" referred to in this chapter, because this replaces a subtle security hazard which many participants could leverage maliciously to one where only the current proposer could leverage it maliciously, and we believe this trade-off is not worth it. This means a direct hash or copy of the most recent BFT finality certificate is insufficient evidence to ensure that specific bitwise copy is canonical or will become canonical in the chain.
```

We are now ready to give a description of a protocol that takes into account the issues described in [Notes on Snap‑and‑Chat](./notes-on-snap-and-chat.md), and that implements [bounded availability](./the-arguments-for-bounded-availability-and-finality-overrides.md#what-is-bounded-availability). We call this the “Crosslink” construction; more precisely the version described here is “Crosslink 2”.

This description will attempt to be self-contained, but [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf) ([arXiv version](https://arxiv.org/pdf/2009.04987.pdf)) is useful background on the general model of Ebb-and-Flow protocols.

## Conventions

“$\star$” is a metavariable for the name of a protocol. We also use it as a wildcard in protocol names of a particular type, for example <span style="white-space: nowrap">“$\star$bc”</span> for the name of some best‑chain protocol.

Protocols are referred to as $\Pi_{\star}$ for a <span style="white-space: nowrap">name “$\star$”.</span> Where it is useful to avoid ambiguity, when referring to a concept defined by $\Pi_{\star}$ we prefix it with <span style="white-space: nowrap">“$\star$‑”.</span>

We do not take synchrony or partial synchrony as an implicit assumption of the communication model; that is, *unless otherwise specified*, messages between protocol participants can be arbitrarily delayed or dropped. A given message is received at most once, and messages are nonmalleably authenticated as originating from a given sender whenever needed by the applicable protocol. Particular subprotocols may require a stronger model.

```admonish info "Background"
For an overview of communication models used to analyze distributed protocols, see [this blog post by Ittai Abraham](https://decentralizedthoughts.github.io/2019-06-01-2019-5-31-models/).
```

```admonish info collapsible=true title="Discussion of incorrect applications of the GST formalization of partial synchrony to continuously operating protocols."
The original context for the definition of the partially synchronous model in [[DLS1988]](https://groups.csail.mit.edu/tds/papers/Lynch/jacm88.pdf) was for “one‑shot” Byzantine Agreement — called “the consensus problem” in that paper. The following argument is used to justify assuming that all messages from the Global Stabilization Time onward are delivered within the <span style="white-space: nowrap">upper time bound $\Delta$:</span>

> Therefore, we impose an additional constraint: For each execution there is a global stabilization time (GST), unknown to the processors, such that the message system respects the upper bound $\Delta$ from time GST onward.
>
> This constraint might at first seem too strong: In realistic situations, the upper bound cannot reasonably be expected to hold forever after GST, but perhaps only for a limited time. However, any good solution to the consensus problem in this model would have an <span style="white-space: nowrap">upper bound $L$</span> on the amount of time after GST required for consensus to be reached; in this case it is not really necessary that the bound $\Delta$ hold forever after time GST, but only up to <span style="white-space: nowrap">time GST $+\; L$.</span> We find it technically convenient to avoid explicit mention of the interval length $L$ in the model, but will instead present the appropriate upper bounds on time for each of our algorithms.

Several subsequent authors applying the partially synchronous model to block chains appear to have forgotten or neglected this context. In particular, the argument depends on the protocol completing soon after GST. Obviously a block‑chain protocol does not satisfy this assumption; it is not a “one‑shot” consensus problem.

This assumption could be removed, but some authors of papers about block‑chain protocols have taken it to be an essential aspect of modelling partial synchrony. I believe this is contrary to the intent of [[DLS1988]](https://groups.csail.mit.edu/tds/papers/Lynch/jacm88.pdf):

> Instead of requiring that the consensus problem be solvable in the GST model, we might think of separating the correctness conditions into safety and termination properties. The safety conditions are that no two correct processors should ever reach disagreement, and that no correct processor should ever make a decision that is contrary to the specified validity conditions. The termination property is just that each correct processor should eventually make a decision. Then we might require an algorithm to satisfy the safety conditions no matter how asynchronously the message system behaves, that is, even if $\Delta$ does not hold eventually. On the other hand, we might only require termination in case $\Delta$ holds eventually. It is easy to see that these safety and termination conditions are [for the consensus problem] equivalent to our GST condition: If an algorithm solves the consensus problem when $\Delta$ holds from time GST onward, then that algorithm cannot possibly violate a safety property even if the message system is completely asynchronous. This is because safety violations must occur at some finite point in time, and there would be some continuation of the violating execution in which $\Delta$ eventually holds.

This argument is correct as stated, i.e. for the one‑shot consensus problem. Subtly, essentially the same argument can be adapted to protocols with *safety* properties that need to be satisfied continuously. However, it cannot correctly be applied to *liveness* properties of non‑terminating protocols. The authors (Cynthia Dwork, Nancy Lynch, and Larry Stockmeyer) would certainly have known this: notice how they carefully distinguish “the GST model” from “partial synchrony”. They cannot plausibly have intended this GST formalization to be applied unmodified to analyze liveness in such protocols, which seems to be common in the block‑chain literature, including in the Ebb-and-Flow paper [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf) and the Streamlet paper [[CS2020]](https://eprint.iacr.org/2020/088.pdf).

The Ebb-and-Flow paper acknowledges the issue by saying “Although in reality, multiple such periods of (a‑)synchrony could alternate, we follow the long‑standing practice in the BFT literature and study only a single such transition.” This is not adequate: “long‑standing practice” notwithstanding, it is not valid in general to infer that properties holding for the first transition to synchrony also apply to subsequent transitions (where the protocol can be in states that would not occur initially), and it is plausible that this inference could fail for real protocols. The Streamlet paper also refers to “periods of synchrony” which indicates awareness of the issue, but then it uses the unmodified GST model in the proofs.

Informally, to solve this issue it is necessary to also prove that existing progress is maintained during periods of asynchrony, and that during such periods the protocol remains in states where it will be able to take advantage of a future period of synchrony to make further progress.

This provides further motivation to avoid taking the GST formalization of partial synchrony as a basic assumption.

Note that the recent result [[CGSW2024]](https://arxiv.org/pdf/2405.10249) does not contradict anything we say here. Although the GST and Unknown Latency models are “equally demanding” in the sense of existence of protocols that satisfy a given goal, this result does not show that the models are equivalent for any specific protocol. In particular the requirements of the “clock‑slowing” technique fail in practice for any protocol involving Proof‑of‑Work.
```

<span style="white-space: nowrap">A $\star$‑execution</span> is the complete set of events (message sends/receives and decisions by protocol participants) that occur in a particular <span style="white-space: nowrap">run of $\Pi_{\star}$</span> from its initiation up to a given time. A prefix of a <span style="white-space: nowrap">$\star$‑execution</span> is also a <span style="white-space: nowrap">$\star$‑execution.</span> Since executions always start from protocol initiation, a strict suffix of a <span style="white-space: nowrap">$\star$‑execution</span> is not a <span style="white-space: nowrap">$\star$‑execution.</span>

Times are modelled as values of a totally ordered type $\Time$ with minimum value $0$. For convenience, we consider all protocol executions to start at time $0$.

```admonish info "Remark"
Although protocols may be nondeterministic, an execution fixes the events that occur and times at which they occur, for the purpose of modeling.

For simplicity, we assume that all events occur at global times in a total ordering. This assumption is not realistic in an asynchronous communication model, but it is not essential to the design or analysis and could be removed: we could use a partial happens-before ordering on events in place of a total ordering on times.
```

<span style="white-space: nowrap">A “$\star$‑node”</span> is a participant in $\Pi_{\star}$ (the protocol may be implicit). <span style="white-space: nowrap">A $\star$‑node</span> is <span style="white-space: nowrap">“honest at time $t$”</span> in a given execution iff it has followed the protocol up to and including <span style="white-space: nowrap">time $t$</span> in that execution.

A time series on <span style="white-space: nowrap">type $U$</span> is a function $S \typecolon \Time \rightarrow U$ assigning a <span style="white-space: nowrap">value of $U$</span> to each time in an execution. By convention, we will write the <span style="white-space: nowrap">time $t$</span> as a superscript: <span style="white-space: nowrap">$S^t = S(t)$.</span>

<span style="white-space: nowrap">A $\star$‑chain</span> is a nonempty sequence of <span style="white-space: nowrap">$\star$‑blocks</span>, starting at the <span style="white-space: nowrap">“genesis block” $\Origin_{\star}$,</span> in which each subsequent block refers to its preceding or “parent block” by a collision‑resistant hash. The “tip” of <span style="white-space: nowrap">a $\star$‑chain</span> is its last element.

For convenience, we conflate <span style="white-space: nowrap">$\star$‑blocks</span> with <span style="white-space: nowrap">$\star$‑chains;</span> that is, we identify a chain with the block at its tip. This is justified because, assuming that the hash function used for parent links is collision‑resistant, there is exactly one <span style="white-space: nowrap">$\star$‑chain</span> corresponding to a <span style="white-space: nowrap">$\star$‑block;</span> and conversely there is exactly one <span style="white-space: nowrap">$\star$‑block</span> at the tip of a <span style="white-space: nowrap">$\star$‑chain.</span>

If $C$ is a <span style="white-space: nowrap">$\star$‑chain,</span> $C \trunc_{\star}^k$ means $C$ with the last $k$ blocks pruned, except that <span style="white-space: nowrap">if $\len(C) \leq k$,</span> the result is the <span style="white-space: nowrap">genesis $\star$‑chain</span> consisting only <span style="white-space: nowrap">of $\Origin_{\star}$.</span>

The block at <span style="white-space: nowrap">depth $k \in \mathbb{N}$</span> in a <span style="white-space: nowrap">$\star$‑chain $C$</span> is defined to be the <span style="white-space: nowrap">tip of $C \trunc_{\star}^k$.</span> Thus the block at <span style="white-space: nowrap">depth $k$</span> in a chain is the last one that cannot be affected by a rollback of <span style="white-space: nowrap">length $k$</span> (this also applies when $\len(C) \leq k$ because the <span style="white-space: nowrap">genesis $\star$‑chain</span> cannot roll back).

```admonish info "Terminology"
Our usage of “depth” is different from [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf), which uses “depth” to refer to what Bitcoin and Zcash call “height”. It also differs by $1$ from the convention for confirmation depths in `zcashd`, where the tip is considered to be at <span style="white-space: nowrap">depth $1$,</span> rather <span style="white-space: nowrap">than $0$.</span>
```

<span id="notation"></span>
For <span style="white-space: nowrap">$\star$‑blocks $B$ and $C$:</span>
* The notation $B \preceq_{\star} C$ means that the <span style="white-space: nowrap">$\star$‑chain</span> with <span style="white-space: nowrap">tip $B$</span> is a prefix of the one with <span style="white-space: nowrap">tip $C$.</span> This includes the <span style="white-space: nowrap">case $B = C$.</span>
* The notation $B \agrees_{\star} C$ means that <span style="white-space: nowrap">either $B \preceq_{\star} C$ or $C \preceq_{\star} B$.</span> That is, <span style="white-space: nowrap">“one of $B$ and $C$</span> is a prefix of the other”. This also includes the <span style="white-space: nowrap">case $B = C$.</span>
* The notation $B \conflicts_{\star} C$ means that <span style="white-space: nowrap">both $B \not\preceq_{\star} C$ and $C \not\preceq_{\star} B$.</span> That is, <span style="white-space: nowrap">“neither of $B$ and $C$</span> is a prefix of the other”.

A function <span style="white-space: nowrap">$S \typecolon I \rightarrow \starblock$ is $\star$‑linear</span> iff <span style="white-space: nowrap">for every $t, u \typecolon I$ where $t \leq u$</span> we have <span style="white-space: nowrap">$S(t) \preceq_{\star} S(u)$.</span> (This definition can be applied to time series <span style="white-space: nowrap">where $I = \Time$,</span> or to sequences of $\star$‑blocks where values of $I$ are indices.)

<span id="linear-prefix-lemma"></span>
```admonish success "Lemma: Linear prefix"
If $A \preceq_{\star} C$ and $B \preceq_{\star} C$ then $A \agrees_{\star} B$.

Proof: The chain of ancestors of $C$ is $\star$-linear, and $A$, $B$ are both on that chain.
```

The notation $[f(X) \text{ for } X \preceq_{\star} Y]$ means the sequence of $f(X)$ for each <span style="white-space: nowrap">$\star$‑block $X$</span> in chain order from genesis up to and <span style="white-space: nowrap">including $Y$.</span> <span style="white-space: nowrap">($X$ is</span> a bound variable within this construct.)

$\TODO$ remove this if not used:

We use $\xlog \preceq \xlog'$ (without a subscript <span style="white-space: nowrap">on $\preceq$)</span> to mean that the transaction ledger $\xlog$ is a prefix <span style="white-space: nowrap">of $\xlog'$.</span> Similarly to $\agrees_{\star}$ above, $\xlog \agrees \xlog'$ means that either <span style="white-space: nowrap">$\xlog \preceq \xlog'$ or $\xlog' \preceq \xlog$;</span> that is, <span style="white-space: nowrap">“one of $\xlog$ and $\xlog'$</span> is a prefix of the other”.

## Views

In the simplest case, a block‑chain protocol $\Pi_{\star}$ provides a single “view” that, <span style="white-space: nowrap">for a given $\star$‑execution,</span> provides <span style="white-space: nowrap">each $\star$‑node</span> with a time series <span style="white-space: nowrap">on $\star$‑chains.</span> More generally a protocol may define several “views” that provide <span style="white-space: nowrap">each $\star$‑node</span> with time series on potentially different chain types.

We model a <span style="white-space: nowrap">$\star$‑view</span> as a function <span style="white-space: nowrap">$V \typecolon \Node \times \Time \rightarrow \starchain$.</span> By convention, we will write the <span style="white-space: nowrap">node index $i$</span> as a subscript and the <span style="white-space: nowrap">time $t$</span> as a superscript: <span style="white-space: nowrap">$V_i^t = V(i, t)$.</span>

<span id="agreement-on-a-view"></span>
```admonish success "Definition: Agreement on a view"
An execution of $\Pi$ has **Agreement** on the view $V \typecolon \Node \times \Time \rightarrow \starchain$ iff <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all $\Pi$ nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have <span style="white-space: nowrap">$V_i^t\, \agrees_{\star} V_j^u$.</span>
```

## Subprotocols

As in Snap‑and‑Chat, we depend on a <span style="white-space: nowrap">BFT protocol $\Pi_{\origbft}$,</span> and a <span style="white-space: nowrap">best‑chain protocol $\Pi_{\origbc}$.</span>

```admonish info
See [this terminology note](./the-arguments-for-bounded-availability-and-finality-overrides.md#terminology-note) for why we do not call $\Pi_{\origbc}$ a “longest‑chain” protocol.
```

We modify $\Pi_{\origbft}$ <span style="white-space: nowrap">(resp. $\Pi_{\origbc}$)</span> to give $\Pi_{\bft}$ <span style="white-space: nowrap">(resp. $\Pi_{\bc}$)</span> by adding structural elements, changing validity rules, and changing the specified behaviour of honest nodes.

A Crosslink 2 node must participate in <span style="white-space: nowrap">both $\Pi_{\bft}$ and $\Pi_{\bc}$;</span> that is, it must maintain a view of the state of each protocol. Acting in more specific roles such as bft‑proposer, bft‑validator, or bc‑block‑producer is optional, but we assume that all such actors are Crosslink 2 nodes.

### Model for BFT protocols (Π<sub>{origbft,bft}</sub>)

<span style="white-space: nowrap">A $\star$bft‑node’s view</span> includes a set of <span style="white-space: nowrap">$\star$bft‑block chains</span> each rooted at a fixed genesis <span style="white-space: nowrap">$\star$bft‑block $\Origin_{\starbft}$.</span> There is a <span style="white-space: nowrap">$\star$bft‑block‑validity</span> rule (specified below), which depends only on the content of the block and its ancestors. A non‑genesis block can only be <span style="white-space: nowrap">$\star$bft‑block‑valid</span> if its parent is $\star$bft‑block‑valid. <span style="white-space: nowrap">A $\star$bft‑valid‑chain</span> is a chain of <span style="white-space: nowrap">$\star$bft‑block‑valid</span> blocks.

Execution proceeds in a sequence of epochs. In each epoch, an honest proposer for that epoch may make a <span style="white-space: nowrap">$\star$bft‑proposal</span>.

A <span style="white-space: nowrap">$\star$bft‑proposal</span> refers to a parent <span style="white-space: nowrap">$\star$bft‑block,</span> and specifies the proposal’s epoch. The content of a proposal is signed by the proposer using a strongly unforgeable signature scheme. We consider the proposal to include this signature. There is a <span style="white-space: nowrap">$\star$bft‑proposal‑validity</span> rule, depending only on the content of the proposal and its parent block, and the validity of the proposer’s signature.

We extend the $\trunc^k$ notation to <span style="white-space: nowrap">$\star$bft‑proposals</span> in the obvious way: <span style="white-space: nowrap">if $k \geq 1$,</span> <span style="white-space: nowrap">$P$ is a $\star$bft‑proposal</span> and $B$ its parent <span style="white-space: nowrap">$\star$bft‑block,</span> then <span style="white-space: nowrap">$P \trunc_{\star\bft}^k \,:= B \trunc_{\star\bft}^{k-1}$.</span>

```admonish info "Terminology"
We will shorten <span style="white-space: nowrap">“$\star$bft‑block‑valid $\star$bft‑block”</span> to <span style="white-space: nowrap">“$\star$bft‑valid‑block”,</span> and <span style="white-space: nowrap">“$\star$bft‑proposal‑valid $\star$bft‑proposal”</span> to <span style="white-space: nowrap">“$\star$bft‑valid‑proposal”.</span>
```

For each epoch, there is a fixed number of voting units distributed between the <span style="white-space: nowrap">$\star$bft‑nodes,</span> which they use to vote for a <span style="white-space: nowrap">$\star$bft‑proposal.</span> We say that a voting unit has been cast for a <span style="white-space: nowrap">$\star$bft‑proposal $P$</span> at a given time in a <span style="white-space: nowrap">$\star$bft‑execution,</span> <span style="white-space: nowrap">if and only if</span> <span style="white-space: nowrap">$P$ is $\star$bft‑proposal‑valid</span> and a ballot <span style="white-space: nowrap">for $P$</span> authenticated by the holder of the voting unit exists at that time.

Using knowledge of ballots cast for a <span style="white-space: nowrap">$\star$bft‑proposal $P$</span> that collectively satisfy a notarization rule at a given time in a <span style="white-space: nowrap">$\star$bft‑execution,</span> and only with such knowledge, it is possible to obtain a valid <span style="white-space: nowrap">$\star$bft‑notarization‑proof $\proof_P$.</span> The notarization rule must require at least a two‑thirds absolute supermajority of voting units <span style="white-space: nowrap">in $P$’s epoch</span> to have been cast <span style="white-space: nowrap">for $P$.</span> It may also require other conditions.

A voting unit is cast non‑honestly for an epoch’s proposal iff:
* it is cast other than by the holder of the unit (due to key compromise or any flaw in the voting protocol, for example); or
* it is double‑cast (i.e. there are at least two ballots casting it for distinct proposals); or
* the holder of the unit following the conditions for honest voting <span style="white-space: nowrap">in $\Pi_{\starbft}$,</span> according to its view, should not have cast that vote.

Note that a unit should be considered to be cast non-honestly in the case of key compromise, because it is then effectively under the control of an adversary. The key compromise may or may not be attributable to another flaw in the protocol, but such a flaw would not be a break of the consensus mechanism per se.

<span id="one-third-bound"></span>
```admonish success "Definition: One‑third bound on non‑honest voting"
An execution of $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property iff for every epoch, *strictly* fewer than one third of the total voting units for that epoch are ever cast non‑honestly.
```

```admonish info
It may be the case that a ballot cast for $P$ is not [in honest view](#in-honest-view) when it is used to create a notarization proof for $P$. Since we are not assuming synchrony, it may also be the case that such a ballot is in honest view but that any given node has not received it (and perhaps will never receive it).

There may be multiple distinct ballots or distinct ballot messages attempting to cast a given voting unit for the same proposal; this is undesirable for bandwidth usage, but it is not necessary to consider it to be non‑honest behaviour for the purpose of security analysis, as long as such ballots are not double‑counted toward the two‑thirds threshold.
```

<span id="one-third-bound-caveat"></span>
```admonish warning "Security caveat"
The **one‑third bound on non‑honest voting** property considers all ballots cast in the entire execution. In particular, it is possible that a validator’s key is compromised and then used to cast its voting units for a proposal of an epoch long finished. If the number of voting units cast non-honestly for any epoch *ever* reaches one third of the total voting units for that epoch during an execution, then the **one‑third bound on non‑honest voting** property is violated for that execution.

Therefore, validator keys of honest nodes must remain secret indefinitely. Whenever a key is rotated, the old key must be securely deleted. For further discussion and potential improvements, see [tfl-book issue #140](https://github.com/Electric-Coin-Company/tfl-book/issues/140).
```

A <span style="white-space: nowrap">$\star$bft‑block</span> consists <span style="white-space: nowrap">of $(P, \proof_P)$</span> re‑signed by the same proposer using a strongly unforgeable signature scheme. It is <span style="white-space: nowrap">$\star$bft‑block‑valid</span> iff:
* $P$ is <span style="white-space: nowrap">$\star$bft‑proposal‑valid</span>; and
* $\proof_P$ is a valid proof that some subset of ballots cast for $P$ are sufficient to satisfy the notarization rule; and
* the proposer’s outer signature <span style="white-space: nowrap">on $(P, \proof_P)$</span> is valid.

<span style="white-space: nowrap">A $\star$bft‑proposal’s</span> parent reference hashes the entire <span style="white-space: nowrap">parent $\star$bft‑block,</span> i.e. proposal, proof, and outer signature.

```admonish info
Neither $\proof_P$ nor the proposer’s outer signature is unique for a <span style="white-space: nowrap">given $P$.</span> The proposer’s outer signature is however third‑party nonmalleable, by definition of a strongly unforgeable signature scheme. An <span style="white-space: nowrap">“honest $\star$bft‑proposal”</span> is a <span style="white-space: nowrap">$\star$bft‑proposal</span> made for a given epoch by a proposer who is honest in that epoch. Such a proposer will only create one proposal and only sign at most once for each epoch, and so there will be at most one <span style="white-space: nowrap">“honestly submitted”</span> <span style="white-space: nowrap">$\star$bft‑block</span> for each epoch.

It is possible for there to be multiple <span style="white-space: nowrap">$\star$bft‑valid‑blocks</span> for the same proposal, with different notarization proofs and/or outer signatures, if the proposer is not honest. However, the property that there will be at most one <span style="white-space: nowrap">“honestly submitted”</span> <span style="white-space: nowrap">$\star$bft‑block</span> for each epoch is important for liveness, even though we cannot guarantee that any particular proposer for an epoch is honest.

$\TODO$ check that we are correctly using this in the liveness analysis.
```

There is an efficiently computable function <span style="white-space: nowrap">$\star\bftlastfinal \typecolon \star\bftblock \to \star\bftblock \union \{\bot\}$.</span> <span style="white-space: nowrap">For a $\star$bft‑block‑valid</span> input <span style="white-space: nowrap">block $C$,</span> this function outputs the last ancestor of $C$ that is final in the <span style="white-space: nowrap">context of $C$.</span>

```admonish info
The chain of ancestors is unambiguously determined because a <span style="white-space: nowrap">$\star$bft‑proposal’s</span> parent reference hashes the entire parent <span style="white-space: nowrap">$\star$bft‑block;</span> each <span style="white-space: nowrap">$\star$bft‑block</span> commits to a proposal; and the parent hashes are collision‑resistant. This holds despite the caveat mentioned above that there may be multiple <span style="white-space: nowrap">$\star$bft‑valid‑blocks</span> for the same proposal.
```

<span style="white-space: nowrap">$\star\bftlastfinal$</span> must satisfy all of the following:

* $\star\bftlastfinal(C) = \bot \iff C$ is not $\star$bft‑block‑valid.
* If $C$ is $\star$bft‑block‑valid, then:
  * $\star\bftlastfinal(C) \preceq_{\starbft} C$ (and therefore it must also be <span style="white-space: nowrap">$\star$bft‑block‑valid);</span>
  * for all <span style="white-space: nowrap">$\star$bft‑valid‑blocks</span> $D$ such that <span style="white-space: nowrap">$C \preceq_{\starbft} D$,</span> <span style="white-space: nowrap">$\star\bftlastfinal(C) \preceq_{\starbft} \star\bftlastfinal(D)$.</span>
* $\star\bftlastfinal(\Origin_{\starbft}) = \Origin_{\starbft}$.

```admonish info
It is correct to talk about the “last final block” of a given chain (that is, each <span style="white-space: nowrap">$\star$bft‑valid-block $C$</span> unambiguously determines a <span style="white-space: nowrap">$\star$bft‑valid-block</span> <span style="white-space: nowrap">$\star\bftlastfinal(C)$)</span>, but it is not correct to refer to a given <span style="white-space: nowrap">$\star$bft‑block</span> as objectively <span style="white-space: nowrap">“$\star$bft‑final”.</span>
```

A particular BFT protocol might need adaptations to fit it into this model <span style="white-space: nowrap">for $\Pi_{\origbft}$,</span> *before* we apply the Crosslink 2 modifications to <span style="white-space: nowrap">obtain $\Pi_{\bft}$.</span> Any such adaptions are necessarily protocol-specific. In particular:
* origbft‑proposal‑validity should correspond to the strongest property of an origbft‑proposal that is objectively and feasibly verifiable from the content of the proposal and its parent origbft‑block at the time the proposal is made. It must include verification of the proposer’s signature.
* origbft‑block‑validity should correspond to the strongest property of an origbft‑block that is objectively and feasibly verifiable from the content of the block and its ancestors at the time the block is added to an origbft‑chain. It should typically include all of the relevant checks from origbft‑proposal‑validity that apply to the created block (or equivalent checks). It must also include verification of the notarization proof and the proposer’s outer signature.
* If a node observes an origbft‑valid block $C$, then it should be infeasible for an adversary to cause a rollback in that node’s view <span style="white-space: nowrap">past $\origbftlastfinal(C)$,</span> and the view of the chain <span style="white-space: nowrap">up to $\origbftlastfinal(C)$</span> should agree with that of all other honest nodes. This is formalized in the next section.

#### Safety of $\Pi_{\starbft}$

The intuition behind the following safety property is that:
* For $\Pi_{\starbft}$ to be safe, it should never be the case that two honest nodes observe (at any time) <span style="white-space: nowrap">$\star$bft‑blocks $B$ and $B'$</span> respectively that they each consider final in some context, but <span style="white-space: nowrap">$B \agrees_{\starbft} B'$</span> does not hold.
* By definition, an honest node observes a <span style="white-space: nowrap">$\star$bft‑block</span> to be final in the context of another <span style="white-space: nowrap">$\star$bft‑block $C$,</span> <span style="white-space: nowrap">iff $B \preceq_{\starbft} \star\bftlastfinal(C)$.</span>

<span id="in-honest-view"></span>
We say that a <span style="white-space: nowrap">$\star$bft‑block</span> is “in honest view” if a party observes it at some time at which that party is honest.

<span id="final-agreement"></span>
```admonish success "Definition: Final Agreement"
An execution of $\Pi_{\starbft}$ has **Final Agreement** iff for all <span style="white-space: nowrap">$\star$bft‑valid blocks $C$</span> in honest view at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$C'$ in honest view</span> at <span style="white-space: nowrap">time $t'$,</span> we have <span style="white-space: nowrap">$\star\bftlastfinal(C) \agrees_{\starbft} \star\bftlastfinal(C')$.</span>
```

Note that it is possible for this property to hold for an execution of a BFT protocol in an asynchronous communication model. As [previously mentioned](#one-third-bound-caveat), if the [**one‑third bound on non‑honest voting**](#one-third-bound) property is *ever* broken at any time in an execution, then it may not be possible to maintain **Final Agreement** from that point on.

```admonish info collapsible=true title="Adapting the Streamlet BFT protocol."
Streamlet as described in [[CS2020]](https://eprint.iacr.org/2020/088.pdf) has three possible states of a block in a player’s view:
* “valid” (but not notarized or final);
* “notarized” (but not final);
* “final”.

By “valid” the Streamlet paper means just that it satisfies the structural property of being part of a block chain with parent hashes. The role of <span style="white-space: nowrap">$\star$bft‑block‑validity</span> in our model corresponds roughly to Streamlet’s “notarized”. It turns out that with some straightforward changes relative to Streamlet, we can identify “origbft‑block‑valid” with “notarized” and consider an origbft‑valid‑chain to only consist of notarized blocks. This is not obvious, but is a useful simplification.

Here is how the paper defines “notarized”:

> When a block gains votes from at least $2n/3$ distinct players, it becomes notarized. A chain is notarized if its constituent blocks are all notarized.

This implies that blocks can be added to chains independently of notarization. However, the paper also says that an honest leader always proposes a block extending from a notarized chain. Therefore, only *notarized* chains really matter in the protocol.

In unmodified Streamlet, the order in which a player sees signatures might cause it to view blocks as notarized out of order. Streamlet’s security analysis is in a synchronous model, and assumes for liveness that any vote will have been received by all players (Streamlet nodes) within two epochs.

In Crosslink 2, however, we need origbft‑block‑validity to be an objectively and feasibly verifiable property. We also would prefer reliable message delivery within bounded time not to be a basic assumption of our communication model. (This does not dictate what assumptions about message delivery are made for particular security analyses.) If we did not make a modification to the protocol to take this into account, then some Crosslink 2 nodes might receive a two‑thirds absolute supermajority of voting messages and consider a BFT block to be notarized, while others might never receive enough of those messages.

Obviously a *proposal* cannot include signatures on itself — but the block formed from it can include proofs about the proposal and signatures. We can therefore say that when a proposal gains a two‑thirds absolute supermajority of signatures, a block is created from it that contains a proof (such as an aggregate signature) that it had such a supermajority. For example, we can have the proposer itself make this proof once it has enough votes, sign the <span style="white-space: nowrap">resulting $(P, \proof_P)$</span> to create a block, then *submit* that block in a separate message. (The proposer has most incentive to do this in order to gain whatever reward attaches to a successful proposal; it can outsource the proving task if needed.) Then the origbft‑block‑validity rule can require a valid supermajority proof, which is objectively and feasibly verifiable. Players that see an origbft‑valid‑block can immediately consider it notarized.

Note that for the liveness analysis to be unaffected, we need to assume that the combined latency of messages, of collecting and aggregating signatures, and of block submission is such that all adapted‑Streamlet nodes will receive a notarized block corresponding to a given proposal (rather than just all of the votes for the proposal) within two epochs. Alternatively we could re‑do the timing analysis.

With this change, “origbft‑block‑valid” and “notarized” do not need to be distinguished.

Streamlet’s finality rule is:

> If in any notarized chain, there are three adjacent blocks with consecutive epoch numbers, the prefix of the chain up to the second of the three blocks is considered final. When a block becomes final, all of its prefix must be final too.

We can straightforwardly express this as an $\origbftlastfinal$ function of a context block $C$, as required by the model:

For an origbft‑valid‑block $C$, $\origbftlastfinal(C)$ is the last origbft‑valid‑block $B \preceq_{\origbft} C$ such that either $B = \Origin_{\origbft}$ or $B$ is the second block of a group of three adjacent blocks with consecutive epoch numbers.

Note that “When a block becomes final, all of its prefix must be final too.” is implicit in the model.
```

### Model for best-chain protocols (Π<sub>{origbc,bc}</sub>)

A node’s view in $\Pi_{\starbc}$ includes a set of <span style="white-space: nowrap">$\star$bc‑block chains</span> each rooted at a fixed <span style="white-space: nowrap">genesis $\star$bc‑block $\Origin_{\starbc}$.</span> There is a <span style="white-space: nowrap">$\star$bc‑block‑validity rule</span> (often described as a collection of <span style="white-space: nowrap">“consensus rules”),</span> depending only on the content of the block and its ancestors. A non‑genesis block can only be <span style="white-space: nowrap">$\star$bc‑block‑valid</span> if its parent is <span style="white-space: nowrap">$\star$bc‑block‑valid.</span> <span style="white-space: nowrap">By “$\star$bc‑valid‑chain”</span> we mean a chain of <span style="white-space: nowrap">$\star$bc‑block‑valid blocks.</span>

```admonish info "Terminology"
The terminology commonly used in the block‑chain community does not distinguish between rules that are part of the consensus protocol proper, and rules required for validity of the economic computation supported by the block chain. Where it is necessary to distinguish, the former can be called “L0” consensus rules, and the latter “L1” consensus rules.
```

The definition of <span style="white-space: nowrap">$\star$bc‑block‑validity</span> is such that it is hard for a block producer to extend a <span style="white-space: nowrap">$\star$bc‑valid‑chain</span> unless they are selected by a random process that chooses a block producer in proportion to their resources with an approximately known and consistent time distribution, subject to some assumption about the total proportion of resources held by honest nodes.

There is a function $\score \typecolon \star\bcvalidchain \to \Score$, with <span style="white-space: nowrap">$<$ a [strict total ordering](https://proofwiki.org/wiki/Definition:Strict_Total_Ordering)</span> <span style="white-space: nowrap">on $\Score$.</span> An honest node will choose one of the <span style="white-space: nowrap">$\star$bc‑valid‑chains</span> with highest score as the <span style="white-space: nowrap">$\star$bc‑best‑chain</span> in its view. Any rule can be specified for breaking ties.

The $\score$ function is required to satisfy $\score(H \trunc_{\!\starbc}^1) \lt \score(H)$ for any non‑genesis <span style="white-space: nowrap">$\star$bc‑valid‑chain $H$.</span>

```admonish info
For a Proof‑of‑Work protocol, the score of a <span style="white-space: nowrap">$\star$bc‑chain</span> should be its accumulated work.
```

Unless an adversary is able to censor knowledge of other chains from a node’s view, it should be difficult to cause the node to switch to a chain with a last common ancestor more than <span style="white-space: nowrap">$\sigma$ blocks</span> back from the tip of its previous <span style="white-space: nowrap">$\star$bc‑best‑chain.</span>

Let $\ch$ be a [view](#views) such that $\ch_i^t$ is <span style="white-space: nowrap">node $i$’s $\star$bc‑best‑chain</span> at time $t$. (This matches the notation used in [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf).) We define $\ch_i^0$ to be $\Origin_{\bc}$.

A <span style="white-space: nowrap">$\star$bc‑valid‑block</span> is assumed to commit to a collection (usually, a sequence) of <span style="white-space: nowrap">$\star$bc‑transactions</span>. Unlike in Crosslink 1 or Snap-and-Chat, we do not need to explicitly model <span style="white-space: nowrap">$\star$bc‑transaction</span> validity or impose any additional constraints on it. The consensus rules applying to <span style="white-space: nowrap">$\star$bc‑transactions</span> are entirely unchanged, including any rules that depend on <span style="white-space: nowrap">$\star$bc‑block</span> height or previous <span style="white-space: nowrap">$\star$bc‑blocks.</span> This is because Crosslink 2 never reorders or selectively “sanitizes” transactions as Snap-and-Chat does. If a bc‑block is included in a Crosslink 2 block chain then its entire parent bc‑block chain is included just as it would have been in $\Pi_{\origbc}$ (only modified by the [structural additions](#structural-additions) described later), so block heights are also preserved.

A “coinbase transaction” is a <span style="white-space: nowrap">$\star$bc‑transaction</span> that only distributes newly issued funds and has no inputs.

Define $\iscoinbaseonlyblock \typecolon \star\bcblock \to \boolean$ so that $\iscoinbaseonlyblock(B) = \true$ iff $B$ has exactly one transaction that is a coinbase transaction.

Each <span style="white-space: nowrap">$\star$bc‑block</span> is summarized by a <span style="white-space: nowrap">$\star$bc‑header</span> that commits to the block. There is a notion of <span style="white-space: nowrap">$\star$bc‑header‑validity</span> that is necessary, but not sufficient, for validity of the block. We will only make the distinction between <span style="white-space: nowrap">$\star$bc‑headers</span> and <span style="white-space: nowrap">$\star$bc‑blocks</span> when it is necessary to avoid ambiguity.

```admonish info collapsible=true title="Header validity for Proof‑of‑Work protocols."
In a Proof‑of‑Work protocol, it is normally possible to check the Proof‑of‑Work of a block using only the header. There is a difficulty adjustment function that determines the target difficulty for a block based on its parent chain. So, checking that the correct difficulty target has been used relies on knowing that the header’s parent chain is valid.

Checking header validity before expending further resources on a purported block can be relevant to mitigating denial‑of‑service attacks that attempt to inflate validation cost.
```

Typically, Bitcoin‑derived best chain protocols do not need much adaptation to fit into this model. The model still omits some details that would be important to implementing Crosslink 2, but distracting for this level of abstraction.

#### Safety of $\Pi_{\starbc}$

We make an assumption on executions of $\Pi_{\origbc}$ that we will call **Prefix Consistency** (introduced in [[PSS2016](https://eprint.iacr.org/2016/454.pdf), section 3.3] as just “consistency”):

<span id="prefix-consistency"></span>
```admonish success "Definition: Prefix Consistency"
An execution of $\Pi_{\starbc}$ has **Prefix Consistency** at confirmation depth $\sigma$, iff <span style="white-space: nowrap">for all times $t \leq u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have that <span style="white-space: nowrap">$\ch_i^t \trunc_{\!\starbc}^\sigma\, \preceq_{\starbc} \ch_j^u$.</span>
```

```admonish info collapsible=true title="Explain the confusion in the literature about what variants of this property are called."
The literature uses the same name, <span style="white-space: nowrap">“common‑prefix property”,</span> for two different properties of very different strength.

[[PSS2016](https://eprint.iacr.org/2016/454.pdf), section 3.3] introduced the stronger variant. That paper first describes the weaker variant, calling it the “common‑prefix property by Garay et al [[GKL2015]](https://link.springer.com/chapter/10.1007/978-3-662-46803-6_10).” Then it explains what is essentially a bug in that variant, and describes the stronger variant which it just calls “consistency”:

> The common‑prefix property by Garay et al [[GKL2015]](https://link.springer.com/chapter/10.1007/978-3-662-46803-6_10), which was already considered and studied by Nakamoto [[Nakamoto2008]](https://citeseerx.ist.psu.edu/viewdoc/summary?doi=10.1.1.221.9986), requires that in any round $r$, the record chains of any two honest players $i$, $j$ agree on all, but potentially the last $T$, records. We note that this property (even in combination with the other two desiderata [of Chain Growth and Chain Quality]) provides quite weak guarantees: even if any two honest parties perfectly agree on the chains, the chain could be completely different on, say, even rounds and odd rounds. We here consider a stronger notion of consistency which additionally stipulates players should be consistent with their “future selves”.
>
> Let $\consistent^T(\view) = 1$ iff for <span style="white-space: nowrap">all rounds $r \leq r'$,</span> and <span style="white-space: nowrap">all players $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest at $\view^r$</span> and <span style="white-space: nowrap">$j$ is honest at $\view^{r'}$,</span> we have that the prefixes of $\mathcal{C}_i^r(\view)$ and $\mathcal{C}_j^{r'}(\view)$ consisting of the first $\ell = |\mathcal{C}_i^r(\view)| - T$ records are identical.

Unfortunately, [[GKL2020]](https://eprint.iacr.org/2014/765.pdf), which is a revised version of [[GKL2015]](https://link.springer.com/chapter/10.1007/978-3-662-46803-6_10), switches to the stronger variant *without changing the name*.

(The [eprint version history](https://eprint.iacr.org/archive/versions/2014/765) may be useful; the change was made in [version 20181013:200033](https://eprint.iacr.org/archive/2014/765/20181013:200033), page 17.)

Note that [GKL2020] uses an adaptive‑corruption model, “meaning that the adversary is allowed to take control of parties on the fly”, and so their wording in Definition 3:
> ... for any pair of <span style="white-space: nowrap">honest players $P_1$, $P_2$</span> adopting the <span style="white-space: nowrap">chains $\mathcal{C}_1$, $\mathcal{C}_2$</span> at rounds $r_1 \leq r_2$ in <span style="white-space: nowrap"><font style="font-variant: small-caps;">view</font>$^{t,n}_{\Pi,\mathcal{A},\mathcal{Z}}$</span> respectively, it holds that <span style="white-space: nowrap">$\mathcal{C}_1^{\trunc k} \preceq \mathcal{C}_2$.</span>

is intended to mean the same as our

> ... <span style="white-space: nowrap">for all times $t \leq u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have that <span style="white-space: nowrap">$\ch_i^t \trunc_{\!\starbc}^\sigma\, \preceq_{\starbc} \ch_j^u$.</span>

The latter is closer to [[PSS2016]](https://eprint.iacr.org/2016/454.pdf).

Incidentally, this property does *not* seem to be mentioned in [[Nakamoto2008]](https://citeseerx.ist.psu.edu/viewdoc/summary?doi=10.1.1.221.9986), contrary to the [PSS2016] authors’ assertion. Maybe implicitly, but it’s a stretch.
```

```admonish info collapsible=true title="Discussion of [GKL2020]’s communication model and network partition."
When **Prefix Consistency** is taken to hold of typical PoW-based block‑chain protocols like Bitcoin (as it often is), this implies that, in the relevant executions, the network of honest nodes is never <span style="white-space: nowrap">partitioned —</span> unless any partition lasts only for a short length of time relative to $\sigma$ block times. <span style="white-space: nowrap">If node $i$</span> is on one side of a full partition and <span style="white-space: nowrap">node $j$</span> on the other, then after <span style="white-space: nowrap">node $i$’s</span> best chain has been extended by more than <span style="white-space: nowrap">$\sigma$ blocks,</span> $\ch_i^t \trunc_{\!\starbc}^\sigma$ will contain information that has no way to get to <span style="white-space: nowrap">node $j$.</span> And even if the partition is incomplete, we cannot guarantee that the Prefix Consistency property will hold for any given pair of nodes.

It might be possible to maintain Prefix Consistency if the honest nodes on one side of the partition knew that they should not continue building on their chain until the partition has healed, but it is unclear how that would be done in general without resorting to a BFT protocol (as opposed to in specific cases like a single node being unable to connect to the rest of the network). Certainly there is no mechanism to explicitly detect and respond to partitions in protocols derived from Bitcoin.

And yet, [[GKL2020]](https://eprint.iacr.org/2014/765.pdf) claims to *prove* Prefix Consistency from other assumptions. So we know that those assumptions must also rule out a long partition between honest nodes. In fact the required assumption is *implicit* in the communication model:
* A synchronous network cannot be partitioned.
* A partially synchronous network <span style="white-space: nowrap">—that is,</span> providing reliable delivery with bounded but <span style="white-space: nowrap">unknown delay—</span> cannot be partitioned for longer than the delay.

We might be concerned that these implicit assumptions are stronger than we would like. In practice, the peer‑to‑peer network protocol of Bitcoin and Zcash attempts to flood blocks to all nodes. This protocol might have weaknesses, but it is not intended to (and plausibly does not) depend on all messages being received. (Incidentally, Streamlet also implicitly floods messages to all nodes.)

Also, Streamlet and many other BFT protocols do *not* assume *for safety* that the network is not partitioned. That is, BFT protocols can be safe in a fully asynchronous communication model with unreliable messaging. That is why we avoid taking synchrony or partial synchrony as an implicit assumption of the communication model, or else we could end up with a protocol with weaker safety properties than $\Pi_{\origbft}$ alone.

This leaves the question of whether the **Prefix Consistency** property is still too strong, even if we do not rely on it for the analysis of safety when $\Pi_{\bft}$ has not been subverted. In particular, if a particular <span style="white-space: nowrap">node $h$</span> is not well-connected to the rest of the network, then that will inevitably affect <span style="white-space: nowrap">node $h$’s</span> security, but should not affect other honest nodes’ security.

Fortunately, it is not the case that disconnecting a single <span style="white-space: nowrap">node $h$</span> from the network causes the security assumption to be voided. The solution is to view $h$ as not honest in that case (even though it would follow the protocol if it could). This achieves the desired effect within the model, because other nodes can no longer rely on <span style="white-space: nowrap">$h$’s honest input.</span> Although viewing $h$ as potentially adversarial might seem conservative from the point of view of other nodes, bear in mind that an adversary could censor an arbitrary subset of incoming and outgoing messages from the node, and this may be best modelled by considering it to be effectively controlled by the adversary.
```

Prefix Consistency compares the <span style="white-space: nowrap">$\sigma$-truncated chain</span> of <span style="white-space: nowrap">some node $i$</span> with the *untruncated* chain of <span style="white-space: nowrap">node $j$.</span> For our analysis of safety of the derived ledgers, we will also need to make an assumption on executions of $\Pi_{\origbc}$ that at <span style="white-space: nowrap">any given time $t$,</span> any two <span style="white-space: nowrap">honest nodes $i$ and $j$</span> **agree** on their confirmed <span style="white-space: nowrap">prefixes —</span> with only the caveat that one may have observed more of the chain than the other. <span style="white-space: nowrap">That is:</span>

<span id="prefix-agreement"></span>
```admonish success "Definition: Prefix Agreement"
An execution of $\Pi_{\starbc}$ has **Prefix Agreement** at confirmation <span style="white-space: nowrap">depth $\sigma$</span> iff it has [**Agreement**](#agreement-on-a-view) on the <span style="white-space: nowrap">view $(i, t) \mapsto \ch_i^t \trunc_{\!\starbc}^\sigma$.</span>
```

```admonish info collapsible=true title="Why are this property, and Prefix Consistency above, stated as unconditional properties of protocol *executions*, rather than as probabilistic assumptions?"
Our security arguments that depend on these properties will all be of the form “in an execution where ⟨safety properties⟩ are not violated, ⟨undesirable thing⟩ cannot happen”.

It is not necessary to involve probability in arguments of this form. Any probabilistic reasoning can be done separately.

In particular, if a statement of this form holds, and ⟨safety properties⟩ are violated with probability <span style="white-space: nowrap">at most $p$</span> under certain conditions, then it immediately follows that under those conditions ⟨undesirable thing⟩ happens with probability <span style="white-space: nowrap">at most $p$.</span> Furthermore, ⟨undesirable thing⟩ can only happen *after* ⟨safety properties⟩ have been violated, because the execution up to that point has been an execution in which ⟨safety properties⟩ are *not* violated.

With few exceptions, involving probability in a security argument is best done only to account for nondeterministic choices in the protocol itself. This is opinionated advice, but a lot of security proofs would likely be simpler if inherently probabilistic arguments were more distinctly separated from unconditional ones.

In the case of the Prefix Agreement property, an alternative approach would be to prove that Prefix Agreement holds with some probability given Prefix Consistency and some other chain properties. This is what [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf) does in its <span style="white-space: nowrap">Theorem 2,</span> which essentially says that under certain conditions Prefix Agreement holds except with <span style="white-space: nowrap">probability $e^{-\Omega(\sqrt{\sigma})}$.</span>

The conclusions that can be obtained from this approach are necessarily probabilistic, and depending on the techniques used, the proof may not be tight; that is, the proof may obtain a bound on the probability of failure that is (either asymptotically or concretely) higher than needed. This is the case for <span>[[NTT2020](https://eprint.iacr.org/2020/1091.pdf), Theorem 2];</span> <span style="white-space: nowrap">footnote 10</span> in that paper points out that the expression for the probability can be asymptotically improved:

> Using the recursive bootstrapping argument developed in <span>[[DKT+2020](https://eprint.iacr.org/2020/601.pdf), Section 4.2],</span> it is possible to bring the error <span style="white-space: nowrap">probability $e^{-\Omega(\sqrt{\sigma})}$</span> as close to an exponential decay as possible. In this context, <span style="white-space: nowrap">for any $\epsilon > 0$,</span> it is possible to find <span style="white-space: nowrap">constants $A_\epsilon$, $a_\epsilon$</span> such that $\Pi_{\lc}(p)$ is secure after <span style="white-space: nowrap">*C*$(\max(\GST, \GAT))$</span> with confirmation time $T_{\confirm} = \sigma$ except with <span style="white-space: nowrap">probability $A_\epsilon\, e^{-a_\epsilon\, \sigma^{1 - \epsilon}}$.</span>

(Here $p$ is the probability that any given node gets to produce a block in any given time slot.)

In fact none of the proofs of security properties for Snap‑and‑Chat depend on the particular expression <span style="white-space: nowrap">$e^{-\Omega(\sqrt{\sigma})}$;</span> for example in the proofs of <span style="white-space: nowrap">Lemma 5</span> and <span style="white-space: nowrap">Theorem 1,</span> this probability just “passes through” the proof from the premisses to the conclusion, because the argument is not probabilistic. The same will be true of our safety arguments.

Talking about what is possible in particular executions has further advantages:
* It sidesteps the issue of how to interpret results in the GST model of partial synchrony, when we do not know what <span style="white-space: nowrap">*C*$(\max(\GST, \GAT))$ is.</span> See also the critique of applying the GST model to block‑chain protocols under [“Discussion of [GKL2020]’s communication model and network partition”](#admonition-discussion-of-gkl2020s-communication-model-and-network-partition) above. (This is not an inherent problem with analyzing the protocol in the partially synchronous setting, but only with inappropriate use of the GST model of that setting.)
* We do not require $\Pi_{\bc}$ to be a Nakamoto‑style Proof‑of‑Work block chain protocol. Some other kind of protocol could potentially satisfy Prefix Consistency and Prefix Agreement.
* It is not clear whether a $e^{-\Omega(\sqrt{\sigma})}$ probability of failure would be concretely adequate. That would depend on the value of $\sigma$ and the constant hidden by the $\Omega$ notation. The asymptotic property using $\Omega$ tells us whether a sufficiently large $\sigma$ *could* be chosen, but we are more interested in what needs to be assumed for a given concrete choice of $\sigma$.
* If a violation of a required safety property occurs *in a given execution*, then the safety argument for Crosslink that depended on the property fails for that execution, regardless of what the probability of that occurrence was. This approach therefore more precisely models the consequences of such violations.
```

```admonish info collapsible=true title="Why, intuitively, should we believe that Prefix Agreement and Prefix Consistency for a large enough confirmation depth hold with high probability for executions of a PoW‑based best‑chain protocol?"
Roughly speaking, the intuition behind both properties is as follows:

Honest nodes are collectively able to find blocks faster than an adversary, and communication between honest nodes is sufficiently reliable that they act as a combined network racing against that adversary. Then by the argument in [[Nakamoto2008]](https://citeseerx.ist.psu.edu/viewdoc/summary?doi=10.1.1.221.9986), modified by [[GP2020]](https://arxiv.org/pdf/1702.02867.pdf) to correct an error in the concrete analysis, a private mining attack that attempts to cause a <span style="white-space: nowrap">$\sigma$‑block</span> rollback will, with high probability, fail for <span style="white-space: nowrap">large enough $\sigma$.</span> A private mining attack is optimal by the argument in [[DKT+2020]](https://arxiv.org/pdf/2005.10484.pdf).

Any further analysis of the conditions under which these properties hold should be done in the context of a <span style="white-space: nowrap">particular $\Pi_{\starbc}$.</span>
```

```admonish info collapsible=true title="Why is the quantification in Prefix Agreement over two *different* times *t* and *t′*?"
This strengthens the security property, relative to quantifying over a single time. The question can then be split into several parts:

1. _**What does the strengthened property mean, intuitively?**_
   Consider the full tree of <span style="white-space: nowrap">$\star$bc‑valid-blocks</span> that honest nodes have considered to be part of their <span style="white-space: nowrap">$\star$bc‑best-chain</span> at any times during the execution. This property holds iff, when we strip off all branches of length up to and including <span style="white-space: nowrap">$\sigma$ blocks,</span> the resulting tree <span style="white-space: nowrap">is $\star$bc‑linear.</span>
2. _**Why is the strengthening needed?**_
   Suppose that time were split into periods such that honest nodes agreed on one chain in odd periods, and a completely different chain in even periods. This would obviously not satisfy the intent, but it would satisfy a version of the property that did not quantify over different <span style="white-space: nowrap">times $t$ and $u$.</span>
3. _**Why should we expect the strengthened property to hold?**_
   If <span style="white-space: nowrap">node $j$</span> were far ahead, <span style="white-space: nowrap">i.e. $u \gg t$,</span> then it is obvious that $\ch_i^t \trunc_{\!\starbc}^\sigma\, \preceq_{\starbc} \ch_j^u \trunc_{\!\starbc}^\sigma$ should hold. Conversely, if node $i$ were far ahead then it is obvious that $\ch_j^u \trunc_{\!\starbc}^\sigma\, \preceq_{\starbc} \ch_i^t \trunc_{\!\starbc}^\sigma$ should hold. The case where $t = u$ is the same as quantifying over a single time. By considering intermediate cases where <span style="white-space: nowrap">$t$ and $u$</span> converge from the extremes or where they diverge from being equal, you should be able to convince yourself that the property holds for any relative values of <span style="white-space: nowrap">$t$ and $u$,</span> in executions of a reasonable best‑chain protocol.
```

## Definition of Crosslink 2

### Parameters

Crosslink 2 is parameterized by a <span style="white-space: nowrap">bc‑confirmation‑depth $\sigma \in \mathbb{N}^+$</span> (as in Snap‑and‑Chat), and also a finalization gap bound $L \in \mathbb{N}^+$ with $L$ significantly greater <span style="white-space: nowrap">than $\sigma$.</span>

Each node $i$ always uses the fixed confirmation <span style="white-space: nowrap">depth $\sigma$</span> to obtain its view of the finalized chain <span style="white-space: nowrap">$\localfin_i^t \typecolon \bcvalidchain$.</span> Unlike in Snap‑and‑Chat or Crosslink 1, this is just a block chain; because we do not need sanitization, there is no need to express it as a log of transactions rather than blocks.

Each node $i$ chooses a potentially different <span style="white-space: nowrap">bc‑confirmation‑depth $\mu \in \mathbb{N}$ where $0 < \mu \leq \sigma$</span> to obtain its view of the bounded‑available ledger at <span style="white-space: nowrap">time $t$</span>, <span style="white-space: nowrap">$(\localba_\mu)_i^t \typecolon \bcvalidchain$.</span> (We make the restriction $\mu \leq \sigma$ because there is no reason to choose a <span style="white-space: nowrap">larger $\mu$.)</span>

```admonish warning "Security caveat"
Choosing $\mu \lt \sigma$</span> is at the node’s own risk and may increase the risk of rollback attacks against $(\localba_\mu)_i^t$ (it does not <span style="white-space: nowrap">affect $\localfin_i^t$).</span> Using small values of $\mu$ is not recommended. The default should <span style="white-space: nowrap">be $\mu = \sigma$.</span>
```

### Stalled Mode

Consider, roughly speaking, the number of bc‑blocks that are not yet finalized at <span style="white-space: nowrap">time $t$</span> (a more precise definition will be given in the section on [$\Pi_{\bc}$ changes from $\Pi_{\origbc}$](#Πbc-changes-from-Πorigbc)). We call this the “finality gap” at <span style="white-space: nowrap">time $t$.</span> Under an assumption about the distribution of bc‑block intervals, if this gap stays roughly constant then it corresponds to the approximate time that transactions take to be finalized after being included in a bc‑block (if they are finalized at all) just prior to <span style="white-space: nowrap">time $t$.</span>

As explained in detail by [The Arguments for Bounded Availability and Finality Overrides](./the-arguments-for-bounded-availability-and-finality-overrides.md), if this bound exceeds a threshold $L$, then it likely signals an exceptional or emergency condition, in which it is undesirable to keep accepting user transactions that spend funds into new bc‑blocks. In practice, $L$ should be *at least* $2\sigma$.

The condition that the network enters in such cases will be called “Stalled Mode”. For a given higher‑level transaction protocol, we can define a policy for which bc‑blocks will be accepted in Stalled Mode. This will be modelled by a predicate <span style="white-space: nowrap">$\isstalledblock \typecolon \bcblock \to \boolean$.</span> A bc‑block for which $\isstalledblock$ returns $\true$ is called a “stalled block”.

```admonish warning "Caution"
A bc‑block producer is only constrained to produce stalled blocks while, roughly speaking, its view of the finalization point is not advancing. In particular an adversary that has subverted the BFT protocol in a way that does *not* keep the finalization point from advancing, can always avoid being constrained by Stalled Mode.
```

The desired properties of stalled blocks and a possible Stalled Mode policy for Zcash are discussed in the [How to block hazards](./the-arguments-for-bounded-availability-and-finality-overrides.md#how-to-block-hazards) section of [The Arguments for Bounded Availability and Finality Overrides](./the-arguments-for-bounded-availability-and-finality-overrides.md).

In practice a node's view of the finalized chain, <span style="white-space: nowrap">$\localfin_i^t$,</span> is likely to lag only a few blocks behind $\localba_{i,\sigma}^t$ (depending on the latency overhead imposed <span style="white-space: nowrap">by $\Pi_{\bft}$),</span> *unless* the chain has entered Stalled Mode. <span style="white-space: nowrap">So when $\mu = \sigma$,</span> the main factor influencing the choice of a given application to use $\localfin_i^t$ or $(\localba_\mu)_i^t$ is not the average latency, but rather the desired behaviour in the case of a finalization stall: i.e. stall immediately, or keep processing user transactions until <span style="white-space: nowrap">$L$ blocks</span> have passed.

### Structural additions

1. Each bc‑header has, in addition to origbc‑header fields, a $\contextbft$ field that commits to a bft‑block.
2. Each bft‑proposal has, in addition to origbft‑proposal fields, a $\headersbc$ field containing a sequence of exactly <span style="white-space: nowrap">$\sigma$ bc‑headers</span> (zero‑indexed, deepest first).
3. Each non‑genesis bft‑block has, in addition to origbft‑block fields, a $\headersbc$ field containing a sequence of exactly <span style="white-space: nowrap">$\sigma$ bc‑headers</span> (zero-indexed, deepest first). The genesis bft‑block has <span style="white-space: nowrap">$\headersbc = \null$.</span>

For a bft‑block or bft‑proposal $B$, define $$
\begin{array}{rl}
\hphantom{\LF(H)}\snapshot(B) &\!\!\!\!:= \begin{cases}
  \Origin_{\bc},&\if B\dot\headersbc = \null \\
  B\dot\headersbc[0] \trunc_{\bc}^1,&\otherwise
\end{cases}
\end{array}
$$
For a bc‑block $H$, define $$
\begin{array}{rl}
\hphantom{\snapshot(B)}\LF(H) &\!\!\!\!:= \bftlastfinal(H\dot\contextbft) \\
                \candidate(H) &\!\!\!\!:= \lastcommonancestor(\snapshotlf{H}, H \trunc_{\bc}^\sigma)
\end{array}
$$

When $H$ is the tip of a node’s bc‑best‑chain, $\candidate(H)$ will give the candidate finalization point, subject to a condition described below that prevents local rollbacks.

```admonish info collapsible=true title="Use of the headers_bc field, and its relation to the ch field in Snap‑and‑Chat."
For a bft‑proposal or <span style="white-space: nowrap">bft‑block $B$,</span> the role of the bc‑chain snapshot referenced by $B\dot\headersbc[0] \trunc_{\bc}^1$ is comparable to the <span style="white-space: nowrap">$\Pi_{\lc}$ snapshot</span> referenced by $B\dot\ch$ in the Snap‑and‑Chat construction from [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf). The motivation for the additional headers is to demonstrate, to any party that sees a bft‑proposal (resp. bft‑block), that the snapshot had been confirmed when the proposal (resp. the block’s proposal) was made.

Typically, a node that is validating an honest bft‑proposal or bft‑block will have seen at least the snapshotted bc‑block (and possibly some of the subsequent bc‑blocks in the $\headersbc$ chain) before. For this not to be the case, the validator’s bc‑best‑chain would have to be more than <span style="white-space: nowrap">$\sigma$ bc‑blocks</span> behind the honest proposer’s bc‑best‑chain at a given time, which would violate the **Prefix Consistency** property <span style="white-space: nowrap">of $\Pi_{\bc}$.</span>

If the headers do not connect to any bc‑valid‑chain known to the validator, then the validator should be suspicious that the proposer might not be honest. It can assign a lower priority to validating the proposal in this case, or simply drop it. The latter option could drop a valid proposal, but this does not in practice cause a problem as long as a sufficient number of validators are properly synced (so that **Prefix Consistency** holds for them).

If the headers *do* connect to a known bc‑valid‑chain, it could still be the case that the whole header chain up to and including $B\dot\headersbc[\sigma-1]$ is not a bc‑valid‑chain. Therefore, to limit denial‑of‑service attacks the validator should first check the Proofs‑of‑Work and difficulty adjustment <span style="white-space: nowrap">—which it can do locally using only the headers—</span> before attempting to download and validate any bc‑blocks that it has not already seen. This is why we include the full headers rather than just the block hashes. Nodes may “trim” (i.e. not explicitly store) headers in a bft‑block that overlap with those referred to by its <span style="white-space: nowrap">ancestor bft‑block(s).</span>
```

```admonish info collapsible=true title="Why is a distinguished value needed for the headers_bc field in the genesis bft‑block?"
It would be conceptually nice for $\Origin_{\bft}\dot\mathsf{headers}[0] \trunc_{\bc}^1$ to refer to $\Origin_{\bc}$, as well as $\Origin_{\bc}\dot\contextbft$ being $\Origin_{\bft}$ so that <span style="white-space: nowrap">$\bftlastfinal(\Origin_{\bc}\dot\contextbft) = \Origin_{\bft}$.</span> That reflects the fact that we know <span style="white-space: nowrap">“from the start”</span> that neither genesis block can be rolled back.

This is not literally implementable using block hashes because it would involve a hash cycle, but we achieve the same effect by defining a $\snapshot$ function that allows us to “patch” $\snapshot(\Origin_{\bft})$ <span style="white-space: nowrap">to be $\Origin_{\bc}$.</span> We do it this way around rather than “patching” the link from a bc‑block to a bft‑block, because the genesis bft‑block already needs a special case since there are not <span style="white-space: nowrap">$\sigma$ bc‑headers</span> available.
```

```admonish info collapsible=true title="Why is the context_bft field needed? Why not use a final_bft field to refer directly to the last final bft‑block before the context block?"
The finality of some bft‑block is only defined in the context of another bft‑block. One possible design would be for a bc‑block to have both $\mathsf{final\_bft}$ and $\contextbft$ fields, so that the finality of $\mathsf{final\_bft}$ could be checked objectively in the context <span style="white-space: nowrap">of $\contextbft$.</span>

However, specifying just the context block is sufficient information to determine its last final ancestor. There would never be any need to give a context block and a final ancestor that is not the last one. The $\bftlastfinal$ function can be computed efficiently for typical BFT protocols. Therefore, having just the $\contextbft$ field is sufficient.
```

### Locally finalized chain

Each node $i$ keeps track of a “locally finalized” <span style="white-space: nowrap">bc‑chain $\localfin_i^t$</span> at <span style="white-space: nowrap">time $t$.</span> Each node’s locally finalized bc‑chain starts <span style="white-space: nowrap">at $\localfin_i^0 = \Origin_{\bc}$.</span> However, this chain state should not be exposed to clients of the node until it [has synced](#syncing-and-checkpoints).

<span id="local-finalization-linearity"></span>
```admonish success "Definition: Local finalization linearity"
Node $i$ has **Local finalization linearity** up to time $t$ iff the time series of <span style="white-space: nowrap">bc‑blocks $\localfin_i^{r \,\leq\, t}$</span> is <span style="white-space: nowrap">[bc‑linear](#notation).</span>
```

When <span style="white-space: nowrap">node $i$’s bc‑best‑chain view</span> is updated <span style="white-space: nowrap">from $\ch_i^s$ to $\ch_i^t$,</span> <span style="white-space: nowrap">the node’s $\localfin_i^t$</span> will become $\candidate(\ch_i^t)$ if and only if this is a descendant <span style="white-space: nowrap">of $\localfin_i^s$.</span> <span style="white-space: nowrap">Otherwise $\localfin_i^t$ will stay at $\localfin_i^s$.</span> This guarantees **Local finalization linearity** by construction.

If when making this update, $\candidate(\ch_i^t) \conflicts \localfin_i^s$ (i.e. $\candidate(\ch_i^t)$ and $\localfin_i^s$ are on different forks), then the node should record a finalization safety hazard. This can only happen if global safety assumptions are violated. Note that **Local finalization linearity** on each node is *not* sufficient for [**Assured Finality**](#assured-finality), but it is necessary.

This can be expressed by the following state update algorithm, where $s$ is the time of the last update and $t$ is the time of the current update:

$$
\updatefin(\mut \localfin_i, \ch_i^t, s): \\
\tab \let N = \candidate(\ch_i^t) \\
\tab \if N \succeq \localfin_i^s \then \\
\tab\tab \localfin_i^t \gets N \\
\tab \else \\
\tab\tab \localfin_i^t \gets \localfin_i^s \\
\tab\tab \if N \not\preceq \localfin_i^s \then \\
\tab\tab\tab \text{record finalization safety hazard}
$$

A safety hazard record should include $\ch_i^t$ and the history of $\localfin_i$ updates including and since the last one that was an ancestor of $N$.

<span id="local-fin-depth"></span>
```admonish success "Lemma: Local fin‑depth"
In any execution of Crosslink 2, for any <span style="white-space: nowrap">node $i$</span> that is honest at <span style="white-space: nowrap">time $t$,</span> there exists a <span style="white-space: nowrap">time $r \leq t$</span> <span style="white-space: nowrap">such that $\localfin_i^t \preceq \ch_i^r \trunc_{\bc}^\sigma$.</span>

Proof: By the definition of $\candidate$ we have $\candidate(\ch_i^u) \preceq \ch_i^u \trunc_{\bc}^\sigma$ for all <span style="white-space: nowrap">times $u$.</span> <span style="white-space: nowrap">Let $r \leq t$</span> be the last time at which <span style="white-space: nowrap">$\localfin_i^t$ changed,</span> or the <span style="white-space: nowrap">genesis time $0$</span> if it has never changed. <span style="white-space: nowrap">Then for $r > 0$</span> we have <span style="white-space: nowrap">$\localfin_i^t = \localfin_i^r = \candidate(\ch_i^r) \preceq \ch_i^r \trunc_{\bc}^\sigma$,</span> and <span style="white-space: nowrap">for $r = 0$</span> we have <span style="white-space: nowrap">$\localfin_i^t = \localfin_i^r = \Origin_{\bc} \preceq \ch_i^r \trunc_{\bc}^\sigma$</span> <span style="white-space: nowrap">(because $\ch_i^0 = \Origin_{\bc}$,</span> and <span style="white-space: nowrap">truncating $\Origin_{\bc}$</span> always <span style="white-space: nowrap">yields $\Origin_{\bc}$).</span>
```

```admonish info collapsible=true title="Why does fin<sub>i</sub> need to be maintained using local state?"
When a node’s view of the bc‑best‑chain reorgs to a different fork (even if the reorg is shorter than $\sigma$ blocks), it may be the case that $\candidate(\ch_i^t)$ rolls back. If [**Final Agreement**](#final-agreement) of $\Pi_{\bft}$ holds up to time $t$, the new snapshot should in that case be an ancestor of the old one. If all is well then this snapshot will subsequently roll forward along the same path. However, we do not want applications using the node to see the temporary rollback.
```

<span id="assured-finality"></span>
[Assured Finality](../../terminology.md#definition-assured-finality) is our main safety goal for Crosslink 2. It is essentially the same goal as **Final Agreement** but applied to nodes’ locally finalized bc‑chains; intuitively it means that honest nodes never see conflicting locally finalized chains. We intend to prove that this goal holds under reasonable assumptions about *either* $\Pi_{\origbft}$ *or* $\Pi_{\origbc}$.

```admonish success "Definition: Assured Finality"
An execution of Crosslink 2 has **Assured Finality** iff <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have <span style="white-space: nowrap">$\localfin_i^t \agrees_{\bc} \localfin_j^u$.</span>
```

Note that if an execution of Crosslink 2 has **Assured Finality**, then all nodes that are honest for that execution have [**Local finalization linearity**](#local-finalization-linearity). That is because the restriction of **Assured Finality** to the <span style="white-space: nowrap">case $i = j$</span> is equivalent to **Local finalization linearity** <span style="white-space: nowrap">for node $i$</span> up to any time at which <span style="white-space: nowrap">node $i$ is honest.</span>

```admonish info collapsible=true title="Why do we need to use candidate(H) rather than snapshot(LF(H))?"
This ensures that the candidate is at least <span style="white-space: nowrap">$\sigma$‑confirmed.</span>

In practice $\candidate(H)$ will rarely differ from $\snapshotlf{H}$, but using the former patches over a potential gap in the safety proof. The [**Last Final Snapshot rule**](#last-final-snapshot-rule) specified later will guarantee that <span style="white-space: nowrap">$\snapshotlf{H} \preceq_{\bc} H$,</span> and this ensures that <span style="white-space: nowrap">$\snapshotlf{H} \agrees_{\bc} H \trunc_{\bc}^\sigma$.</span> However, the depth of $\snapshotlf{H}$ relative to $H$ is not guaranteed to be $\geq \sigma$. For the proof we will need <span style="white-space: nowrap">$\candidate(H) \preceq_{\bc} H \trunc_{\bc}^\sigma$,</span> so that we can use the [**Local fin‑depth lemma**](#local-fin-depth) together with [**Prefix Agreement**](#prefix-agreement) of $\Pi_{\bc}$ at confirmation depth $\sigma$ to prove [**Assured Finality**](#assured-finality).

An alternative would be to change the [**Last Final Snapshot rule**](#last-final-snapshot-rule) to directly require <span style="white-space: nowrap">$\snapshotlf{H} \preceq_{\bc} H \trunc_{\bc}^\sigma$.</span>

$\TODO$ Choose between these options based on what works well for the security proofs and finalization latency.
```

### Locally bounded‑available chain

Define the locally bounded‑available chain on node $i$ for bc‑confirmation‑depth $\mu$, as $$
(\localba_\mu)_i^t = \begin{cases}
  \ch_i^t \trunc_{\bc}^\mu, &\if \localfin_i^t \preceq \ch_i^t \trunc_{\bc}^\mu \\
  \localfin_i^t, &\otherwise
\end{cases}
$$

Like the locally finalized bc‑chain, this chain state should not be exposed to clients of the node until it [has synced](#syncing-and-checkpoints).

<span id="ledger-prefix-property"></span>
```admonish success "Theorem: Ledger prefix property"
For any node $i$ that is honest at <span style="white-space: nowrap">time $t$,</span> and any confirmation <span style="white-space: nowrap">depth $\mu$,</span> <span style="white-space: nowrap">$\localfin_i^t \preceq (\localba_\mu)_i^t$.</span>

Proof: By construction of $(\localba_\mu)_i^t$.</span>
```

<span id="local-ba-depth"></span>
```admonish success "Lemma: Local ba‑depth"
In any execution of Crosslink 2, for any <span style="white-space: nowrap">confirmation depth $\mu \leq \sigma$</span> and any <span style="white-space: nowrap">node $i$</span> that is honest at <span style="white-space: nowrap">time $t$,</span> there exists a <span style="white-space: nowrap">time $r \leq t$</span> <span style="white-space: nowrap">such that $(\localba_\mu)_i^t \preceq_{\bc} \ch_i^r \trunc_{\bc}^\mu$.</span>

Proof: Either $(\localba_\mu)_i^t = \localfin_i^t$, in which case the result follows by the [**Local fin‑depth lemma**](#local-fin-depth) since $\mu \leq \sigma$, or $(\localba_\mu)_i^t = \ch_i^t \trunc_{\bc}^\mu$ in which case it follows trivially with $r = t$.
```

Our security goal for $\localba_\mu$ will be [**Agreement**](#agreement-on-a-view) on $\localba_{\mu}$</span> as already defined.

```admonish info collapsible=true title="Why is the ‘otherwise’ case in the definition of (ba<sub>μ</sub>)<sub>i</sub><sup>t</sup> necessary?"
Assume for this discussion that $\Pi_{\bc}$ uses PoW.

Depending on the value of $\sigma$, the timestamps of bc‑blocks, and the difficulty adjustment rule, it can be the case that if $\localfin_i$ switches to a different fork, the difficulty on that fork is greater than on the chain of the previous snapshot. Then, the new bc‑chain could reach a higher score than the previous chain in fewer than <span style="white-space: nowrap">$\sigma$ blocks</span> from the fork point, and so $\ch_i^t \trunc_{\bc}^\mu$ might not be a descendant of $\localfin_i^t$ (which is more likely <span style="white-space: nowrap">if $\mu = \sigma$).</span> This can occur even when all safety assumptions are satisfied.

For [Zcash’s difficulty adjustment algorithm](https://zips.z.cash/protocol/protocol.pdf#diffadjustment), the difficulty of each block is adjusted based on the median timestamps and difficulty target thresholds over a range of <span style="white-space: nowrap">$\mathsf{PoWAveragingWindow} = 17$ blocks,</span> where each median is taken over <span style="white-space: nowrap">$\mathsf{PoWMedianBlockSpan} = 11$ blocks.</span> Other damping factors and clamps are applied in order to prevent instability and to reduce the influence that adversarially chosen timestamps can have on difficulty adjustment. This makes it unlikely that an adversary could gain a significant advantage by manipulating the difficulty adjustment. So it is safe to use $\localfin_i^t$ in this case: even though it does not have <span style="white-space: nowrap">$\mu$ confirmations</span> relative to <span style="white-space: nowrap">$\ch_i^t$,</span> it does have at least the required amount of work “on top” of it.

Defining $(\localba_\mu)_i^t$ this way also has the advantage of making the proof of the **Ledger prefix property** trivial.
```

### Syncing and checkpoints

It is recommended that node implementations “bake in” a checkpointed <span style="white-space: nowrap">bft‑block $B_{\checkpoint}$</span> to each released version, and that node $i$ should only expose <span style="white-space: nowrap">$\localfin_i^t$ and $(\localba_\mu)_i^t$</span> to its clients once it is “synced”, that is:
* $B_{\checkpoint} \preceq_{\bft} \LF(\ch_i^t)$;  and
* $\snapshot(B_{\checkpoint}) \preceq_{\bc} \localfin_i^t$;  and
* the timestamp of $\localfin_i^t$ is within some threshold of the current time.

### Π<sub>bft</sub> changes from Π<sub>origbft</sub>

#### Π<sub>bft</sub> proposal and block validity

<span id="genesis-bft-block-rule"></span>**Genesis bft‑block rule:** $\Origin_{\bft}$ is bft‑block‑valid.

A bft‑proposal (resp. non‑genesis bft‑block) $B$ is bft‑proposal‑valid (resp. bft‑block‑valid) iff all of the following hold:

* <span id="inherited-origbft-rules"></span>**Inherited origbft rules:** The corresponding origbft‑proposal‑validity (resp. origbft‑block‑validity) rules hold for $B$.
* <span id="linearity-rule"></span>**Linearity rule:** $\snapshot(B \trunc_{\bft}^1) \preceq_{\bc} \snapshot(B)$.
* <span id="tail-confirmation-rule"></span>**Tail Confirmation rule:** $B\dot\headersbc$ form the $\sigma$‑block tail of a bc‑valid‑chain.

<span id="parent-rule"></span>
The “corresponding validity rules” are assumed to include the **Parent rule** that $B$’s parent is bft‑valid.

Note: origbft‑block‑validity rules may be different to origbft‑proposal‑validity rules. For example, in adapted Streamlet, a origbft‑block needs evidence that it was voted for by a supermajority, and an origbft‑proposal doesn’t. Such differences also apply to bft‑block‑validity vs bft‑proposal‑validity.

```admonish info collapsible=true title="Why have validity rules been separated from the honest voting condition below?"
The reason to separate the validity rules from the honest voting condition, is that the validity rules are objective: they don’t depend on an observer’s view of the bc‑best‑chain. Therefore, they can be checked independently of validator signatures. Even a proposal voted for by 100% of validators will not be considered bft‑proposal‑valid by other nodes unless it satisfies the above rules. If more than two thirds of voting units are cast for an **invalid** proposal, something is seriously *and visibly* wrong; in any case, the block will not be accepted as a bft‑valid‑block. Importantly, a purportedly valid bft‑block will not be recognized as such by *any* honest Crosslink 2 node even if it includes a valid notarization proof, if it does not meet other bft‑block‑validity rules.

This is essential to making the finalized chain safe against a flaw in $\Pi_{\bft}$ or its security assumptions (even, say, a complete break of the validator signature algorithm), as long as $\Pi_{\bc}$ remains safe.
```

```admonish info collapsible=true title="What does the **Linearity rule** do?"
This rule is key to combining simplicity with strong security properties in Crosslink 2. It essentially says that, in a given bft‑valid‑chain, the snapshots pointed to by blocks in that chain cannot roll back.

This allows the informal safety argument for Crosslink 2 to be rather intuitive.

Informally, if $\Pi_{\bft}$ has [**Final Agreement**](#final-agreement), then all nodes see only one consistent [bft‑linear](#notation) chain (restricting to bft‑blocks that are final in the context of some bft‑block [in honest view](#in-honest-view)). Within such a bft‑chain, the [**Linearity rule**](#linearity-rule) ensures *by construction* that the sequence of referenced bc‑chain snapshots is [bc‑linear](#notation). This implies [**Assured Finality**](#assured-finality), without needing to assume any safety property of $\Pi_{\bc}$.

We will *also* be able to prove safety of the finalized snapshots based only on safety of $\Pi_{\bc}$ (for a confirmation depth of $\sigma$), without needing to assume any safety property of $\Pi_{\bft}$. Informally, that is because each node sees each candidate final snapshot at a given time as a $\sigma$-confirmed prefix of its bc‑best‑chain at that time (this can be proven based on the [**Last Final Snapshot rule**](#Πbc-changes-from-Πorigbc) and the fact that a snapshot includes $\sigma$ subsequent headers), and [**Prefix Agreement**](#prefix-agreement) implies that honest nodes agree on this prefix. We will leave a more detailed argument until after we have presented the <span style="white-space: nowrap">[$\Pi_{\bc}$ changes from $\Pi_{\origbc}$](#Πbc-changes-from-Πorigbc).</span>

The [**Linearity rule**](#linearity-rule) replaces the “Increasing Score rule” used in Crosslink 1. The Increasing Score rule required that each snapshot in a bft‑valid‑chain either be the same snapshot, or a higher-scoring snapshot to that of its parent block. Since scores strictly increase within a bc‑valid‑chain, the [**Linearity rule**](#linearity-rule) *implies* the Increasing Score rule. It retains the same or stronger positive effects:

* It prevents potential attacks that rely on proposing a bc‑valid‑chain that forks from a much earlier block. This is necessary because the difficulty (or stake threshold) at that point could have been much lower.
* It limits the extent of disruption an adversary can feasibly cause to the bounded‑available <span style="white-space: nowrap">chain $\ch_i$,</span> *even if* it has <span style="white-space: nowrap">subverted $\Pi_{\bft}$.</span> Informally, because the finalized chain *is* a $\Pi_{\bc}$ chain, its safety is no worse than $\Pi_{\bc}$ alone for a rollback of *any* depth.
* It ensures that either progress is made (the snapshot advances relative to that of the parent bft‑block), *or* there is no further validation that needs to be done for the snapshot because it was already validated.

Note that the adversary could take advantage of an “accidental” fork and start its attack from the base of that fork, so that not all of this work is done by it alone. This is also possible in the case of a standard “private mining” attack, and is not so much of a problem in practice because accidental forks are expected to be short. In any case, <span style="white-space: nowrap">$\sigma$ should</span> be chosen to take it into account.

The [**Linearity rule**](#linearity-rule) is also critical to removing the need for one of the most complex elements of Snap‑and‑Chat and Crosslink 1, “sanitization”. In those protocols, because bc‑chain snapshots could be unrelated to each other, it was necessary to sanitize the chain formed from these snapshots to remove transactions that were contextually invalid (e.g. because they double‑spend). The negative consequences of this are described in [Notes on Snap‑and‑Chat](./notes-on-snap-and-chat.md); avoiding it is much simpler.

The linearity property is intentionally always relative to the snapshot of the parent bft‑block, even if it is not final in the context of the current bft‑block. This is because the rule needs to hold if and when it becomes final in the context of some descendant bft‑block.

$\TODO$ PoS Desideratum: we want leader selection with good security / performance properties that will be relevant to this rule. (Suggested: [PoSAT](https://arxiv.org/pdf/2010.08154.pdf).)
```

```admonish info collapsible=true title="Why does the **Linearity rule** allow keeping the same snapshot as the parent?"
This is necessary in order to preserve liveness of $\Pi_{\bft}$ relative <span style="white-space: nowrap">to $\Pi_{\origbft}$.</span> Liveness of $\Pi_{\origbft}$ might require honest proposers to make proposals at a minimum rate. That requirement could be consistently violated if it were not always possible to make a valid proposal. But given that it is allowed to repeat the same snapshot as in the parent bft‑block, neither the [**Linearity rule**](#linearity-rule) nor the [**Tail Confirmation rule**](#tail-confirmation-rule) can prevent making a valid proposal — and all other rules of $\Pi_{\bft}$ affecting the ability to make valid proposals are the same as <span style="white-space: nowrap">in $\Pi_{\origbft}$.</span> <span style="white-space: nowrap">(In principle,</span> changes to voting in $\Pi_{\bft}$ could also affect its liveness; we’ll discuss that in the liveness proof later.)

For example, Streamlet requires three notarized blocks *in consecutive epochs* in order to finalize <span style="white-space: nowrap">a block [[CS2020](https://eprint.iacr.org/2020/088.pdf), section 1.1].</span> Its proof of liveness depends on the assumption that in each epoch for which the leader is honest, that leader will make a proposal, and that during a “period of synchrony” this proposal will be received by <span style="white-space: nowrap">every node [[CS2020](https://eprint.iacr.org/2020/088.pdf), section 3.6].</span> This argument can also be extended to adapted‑Streamlet.

We could alternatively have allowed to always make a “null” proposal, rather than to always make a proposal with the same snapshot as the parent. We prefer the latter because the former would require specifying the rules for null proposals <span style="white-space: nowrap">in $\Pi_{\origbft}$.</span>

As a clarification, no BFT protocol that uses leader election can *require* a proposal in each epoch, because the leader might be dishonest. The above issue concerns liveness of the protocol when assumptions about the attacker’s share of bft‑validators or stake are met, so that it can be assumed that sufficiently long periods with enough honest leaders to make progress <span style="white-space: nowrap">(5 consecutive</span> epochs in the case of Streamlet), will occur with significant probability.
```

#### Π<sub>bft</sub> block finality in context

The finality rule for bft‑blocks in a given context is unchanged from origbft‑finality. That is, $\bftlastfinal$ is defined in the same way as $\origbftlastfinal$ (modulo referring to bft‑block‑validity and $\Origin_{\bft}$).

#### Π<sub>bft</sub> honest proposal

An honest proposer of a bft‑proposal $P$ chooses $P\dot\headersbc$ as the $\sigma$‑block tail of its bc‑best‑chain, provided that it is consistent with the [**Linearity rule**](#linearity-rule). If it would not be consistent with that rule, it sets $P\dot\headersbc$ to the same $\headersbc$ field as $P$’s parent bft‑block. It does not make proposals until its bc‑best‑chain is at least $\sigma + 1$ blocks long.

```admonish info collapsible=true title="Why σ + 1?"
If the length were less than $\sigma + 1$ blocks, it would be impossible to construct the $\headersbc$ field of the proposal.

Note that when the length of the proposer’s bc‑best‑chain is exactly $\sigma + 1$ blocks, the snapshot must be of $\Origin_{\bc}.$ But this does not violate the [**Linearity rule**](#linearity-rule), because $\Origin_{\bc}$ matches the previous snapshot by $\Origin_{\bft}$.
```

```admonish info collapsible=true title="How is it possible that the [**Linearity rule**](#linearity-rule) would not be satisfied by choosing headers from an honest proposer’s bc‑best‑chain?"
As in the answer to [Why is the ‘otherwise’ case in the definition of $(\localba_\mu)_i^t$ necessary?](#admonition-why-is-the-otherwise-case-in-the-definition-of-bai%CE%BCt-necessary) above, after a reorg on the bc‑chain, the $\sigma$-confirmed block on the new chain might not be a descendant of the $\sigma$-confirmed block on the old chain, which could break the [**Linearity rule**](#linearity-rule).
```

#### Π<sub>bft</sub> honest voting

An honest validator considering a proposal $P$, first updates its view of both subprotocols with the bc‑headers given in $P\dot\headersbc$, downloading bc‑blocks for these headers and checking their bc‑block‑validity.

For each downloaded bc‑block, the bft‑chain referenced by its $\contextbft$ field might need to be validated if it has not been seen before.

```admonish info collapsible=true title="Wait what, how much validation is that?"
In general the entire referenced bft‑chain needs to be validated, not just the referenced block — and for each bft‑block, the bc‑chain in $\headersbc$ needs to be validated, and so on recursively. If this sounds overwhelming, note that:
* We should check the requirement that a bft‑valid‑block must have been voted for by a two‑thirds absolute supermajority of validators, and any other *non‑recursive* bft‑validity rules, *first*.
* Before validating a bc‑chain referenced by a $\headersbc$ field, we check that it connects to an already-validated bc‑chain and that the Proofs‑of‑Work are valid. This implies that the amount of bc‑block validation is constrained by how fast the network can find valid Proofs‑of‑Work.
* The [**Linearity rule**](#linearity-rule) reduces the worst‑case validation effort, by ensuring that only one bc‑chain needs to be validated for any bft‑chain. Assuming safety of $\Pi_{\bft}$ and that the adversary does not have an overwhelming advantage in computing the Proof‑of‑Work, this is effectively only one bc‑chain overall with, at most, short side branches.

In summary, the order of validation is important to avoid denial‑of‑service — but it already is in Bitcoin and Zcash.
```

After updating its view, the validator will vote for a proposal $P$ *only if*:

* <span id="valid-proposal-criterion"></span>**Valid proposal criterion:** it is bft‑proposal‑valid, and
* <span id="confirmed-best-chain-criterion"></span>**Confirmed best‑chain criterion:** $\snapshot(P)$ is part of the validator’s bc‑best‑chain at a bc‑confirmation‑depth of at least $\sigma$.

```admonish info collapsible=true title="Blocks in a bc‑best‑chain are by definition bc‑block‑valid. If we’re checking the **Confirmed best‑chain criterion**, why do we need to have separately checked that the blocks referenced by the headers are bc‑block‑valid?"
The **Confirmed best‑chain criterion** is quite subtle. It ensures that $\snapshot(P) = P\dot\headersbc[0] \trunc_{\bc}^1$ is bc‑block‑valid and has $\sigma$ bc‑block‑valid blocks after it in the validator’s bc‑best‑chain. However, it need not be the case that $P\dot\headersbc[\sigma-1]$ is part of the validator’s bc‑best‑chain after it updates its view. That is, the chain could fork after $\snapshot(P)$.

The bft‑proposal‑validity rule must be objective; it can’t depend on what the validator’s bc‑best‑chain is. The validator’s bc‑best‑chain *may* have been updated to $P\dot\headersbc[\sigma-1]$ (if it has the highest score), but it also may not.

However, if the validator’s bc‑best‑chain *was* updated, that makes it more likely that it will be able to vote for the proposal.

In any case, if the validator does not check that all of the blocks referenced by the headers are bc‑block‑valid, then its vote may be invalid.
```

```admonish info collapsible=true title="How does this compare to Snap‑and‑Chat?"
Snap‑and‑Chat already had the voting condition:
> An honest node only votes for a proposed BFT block $B$ if it views $B\dot\ch$ as confirmed.

but it did *not* give the headers potentially needed to update the validator’s view, and it did not require a proposal to be for an *objectively confirmed* snapshot as a matter of validity.

If a Crosslink‑like protocol were to require an objectively confirmed snapshot but without including the bc‑headers in the proposal, then validators would not immediately know which bc‑blocks to download to check its validity. This would increase latency, and would be likely to lead proposers to be more conservative and only propose blocks that they think will *already* be in at least a two‑thirds absolute supermajority of validators’ best chains.

That is, showing $P\dot\headersbc$ to all of the validators is advantageous to the proposer, because the proposer does not have to guess what blocks the validators might have already seen. It is also advantageous for the protocol goals in general, because it improves the trade‑off between finalization latency and security.
```

### Π<sub>bc</sub> changes from Π<sub>origbc</sub>

#### Π<sub>bc</sub> block validity

<span id="genesis-bc-block-rule"></span>**Genesis bc‑block rule:** For the genesis bc‑block we must have <span style="white-space: nowrap">$\Origin_{\bc}\dot\contextbft = \Origin_{\bft}$,</span> and therefore <span style="white-space: nowrap">$\bftlastfinal(\Origin_{\bc}\dot\contextbft) = \Origin_{\bft}$.</span>

A bc‑block $H$ is bc‑block‑valid iff all of the following hold:
* <span id="inherited-origbc-rules"></span>**Inherited origbc rules:** $H$ satisfies the corresponding origbc‑block‑validity rules.
* <span id="valid-context-rule"></span>**Valid context rule:** $H\dot\contextbft$ is bft‑block‑valid.
* <span id="extension-rule"></span>**Extension rule:** $\LF(H \trunc_{\bc}^1) \preceq_{\bft} \LF(H)$.
* <span id="last-final-snapshot-rule"></span>**Last Final Snapshot rule:** $\snapshotlf{H} \preceq_{\bc} H$.
* <span id="finality-depth-rule"></span>**Finality depth rule:** Define: $$
\finalitydepth(H) := \height(H) - \height(\snapshotlf{H})
$$ Then either $\finalitydepth(H) \leq L$ or <span style="white-space: nowrap">$\isstalledblock(H)$.</span>

```admonish info collapsible=true title="Explain the definition of finality‑depth."
The finality depth must be objectively defined, since it is used in a consensus rule. Therefore it should measure the height of $H$ relative to $\snapshotlf{H}$, which is an objectively defined function of $H$, rather than relative to $\localfin_i^t$. (These will only differ for $H = \ch_i^t$ when node $i$ has just reorged, and only then in corner cases.)

Note that the [**Last Final Snapshot rule**](#last-final-snapshot-rule) ensures that it is meaningful to simply use the difference in heights, since $\snapshotlf{H} \preceq_{\bc} H$.
```

#### Π<sub>bc</sub> contextual validity

The consensus rule changes above are all non-contextual. Modulo these changes, contextual validity <span style="white-space: nowrap">in $\Pi_{\bc}$</span> is the same as <span style="white-space: nowrap">in $\Pi_{\origbc}$.</span>

#### Π<sub>bc</sub> honest block production

An honest producer of a <span style="white-space: nowrap">bc‑block $H$</span> must follow the consensus rules under [$\Pi_{\bc}$ block validity](#Πbc-block-validity) above. In particular, it must produce a stalled block if required to do so by the [**Finality depth rule**](#finality-depth-rule).

To <span style="white-space: nowrap">choose $H\dot\contextbft$,</span> the producer considers a subset of the tips of bft‑valid‑chains in its view: $$
\{ T : T \text{ is bft‑block‑valid and } \LF(H \trunc_{\bc}^1) \preceq_{\bft} \bftlastfinal(T) \}
$$ It chooses one of the longest of these chains, $C$, breaking ties by maximizing <span style="white-space: nowrap">$\score(\snapshot(\bftlastfinal(C)))$,</span> and if there is still a tie then by taking $C$ with the smallest hash.

The honest block producer then sets <span style="white-space: nowrap">$H\dot\contextbft$ to $C$.</span>

```admonish warning "Attention"
An honest bc‑block‑producer must not use information from the BFT protocol, other than the specified consensus rules, to decide which bc‑valid‑chain to follow. The specified consensus rules that depend on $\Pi_{\bft}$ have been carefully constructed to preserve safety of $\Pi_{\bc}$ relative <span style="white-space: nowrap">to $\Pi_{\origbc}$.</span> Imposing any additional constraints could potentially allow an adversary that is able to <span style="white-space: nowrap">subvert $\Pi_{\bft}$,</span> to influence the evolution of the bc‑best‑chain in ways that are not considered in the safety argument.
```

```admonish info collapsible=true title="Why not choose *T*  such that *H* ⌈<sup>1</sup><sub>bc</sub> . context_bft  ⪯<sub>bft</sub>  bft‑last‑final(*T* )?"
The effect of this would be to tend to more often follow the last bft‑block seen by the producer of the parent bc‑block, if there is a choice. It is not always possible to do so, though: the resulting set of candidates <span style="white-space: nowrap">for $C$</span> might be empty.

Also, it is not clear that giving the parent bc‑block‑producer the chance to “guide” what bft‑block should be chosen next is beneficial, since that producer might be adversarial and the resulting incentives are difficult to reason about.
```

```admonish info collapsible=true title="Why choose the longest *C*, rather than the longest bft‑last‑final(*C* )?"
We could have instead chosen $C$ to maximize the length <span style="white-space: nowrap">of $\bftlastfinal(C)$.</span> The rule we chose follows Streamlet, which builds on the longest notarized chain, not the longest finalized chain. This may call for more analysis specific to the chosen BFT protocol.
```

```admonish info collapsible=true title="Why this tie‑breaking rule?"
Choosing the bft‑chain that has the last final snapshot with the highest score, tends to inhibit an adversary’s ability to finalize its own chain if it has a lesser score. (If it has a greater score, then it has already won a hash race and we cannot stop the adversary chain from being finalized.)
```

For discussion of potentially unifying the roles of bc‑block producer and bft‑proposer, see [What about making the bc‑block‑producer the bft‑proposer?](./potential-changes.md#what-about-making-the-bcblockproducer-the-bftproposer) in [Potential changes to Crosslink](./potential-changes.md).

At this point we have completed the definition of Crosslink 2. In [Security Analysis of Crosslink 2](./security-analysis.md), we will prove it secure.
