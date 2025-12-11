# Security Analysis of Crosslink 2

```admonish warning "Attention"
This section is an almost direct paste from a section found in `https://github.com/Electric-Coin-Company/tfl-book` for the purposes of precisely committing to a (potentially incomplete, confusing, or inconsistent) precise set of book contents for a security design review.

We believe this respects the license of the other repository. However, the other authors of the TFL-Book, Daira Hopwood and Jack Grigg have not reviewed and do not endorse anything found in this repository in terms of design or otherwise.
```

This document analyzes the security of Crosslink 2 in terms of liveness and safety. It assumes that you have read [The Crosslink 2 Construction](./construction.md) which defines the protocol.

## Liveness argument

First note that Crosslink 2 *intentionally* sacrifices **availability** if there is a long finalization stall.

```admonish info
This is technically independent of the other changes; you can omit the [**Finality depth rule**](./construction.md#finality-depth-rule) and the protocol would still have security advantages over Snap‑and‑Chat, as well as being much simpler and solving its “spending from finalized outputs” issue. In that case the incentives to “pull” the finalization point forward to include new final bft‑blocks would be weaker, but honest bc‑block‑producers would still do it.
```

It would still be a bug if there were any situation in which $\Pi_{\bc}$ failed to be **live**, though, because that would allow [tail‑thrashing attacks](./the-arguments-for-bounded-availability-and-finality-overrides.md#tail-thrashing-attacks).

Crosslink 2 involves a bidirectional dependence between $\Pi_{\mathsf{bft}}$ and $\Pi_{\mathsf{bc}}$. The Ebb‑and‑Flow paper [[NTT2020]](https://eprint.iacr.org/2020/1091.pdf) argues in Appendix E ("Bouncing Attack on Casper FFG") that it can be more difficult to reason about liveness given a bidirectional dependence between protocols:

> To ensure consistency among the two tiers [of Casper FFG], the fork choice rule of the blockchain is modified to always respect ‘the justified checkpoint of the greatest <span style="white-space: nowrap">height [*]’ [[22]](https://arxiv.org/abs/1710.09437).</span> There is thus a bidirectional interaction between the block proposal and the finalization layer: blocks proposed by the blockchain are input to finalization, while justified checkpoints constrain future block proposals. This bidirectional interaction is intricate to reason about and a gateway for liveness attacks.

```admonish info
[*] The quotation changes this to “[depth]”, but our terminology is consistent with Ethereum here and not with [NTT2020]’s idiosyncratic use of “depth” to mean “block height”.
```

The argument is correct as far as it goes. The main reason why this does not present any great difficulty to proving liveness of Crosslink, is due to a fundamental difference from Casper FFG: in Crosslink 2 the fork‑choice rule of $\Pi_{\bc}$ is not modified.

Let $\mathcal{S}$ be the subset of bc‑blocks $\{ B : \iscoinbaseonlyblock(B) \text{ and } \isstalledblock(B) \}$. Assume that $\mathcal{S}$ is such that a bc‑block‑producer can always produce a block in $\mathcal{S}$.

In that case it is straightforward to convince ourselves that the additional bc‑block‑validity rules are never an obstacle to producing a new bc‑block in $\mathcal{S}$:
* The changes to the definition of contextual validity do not interfere with liveness, since they do not affect coinbase transactions, and therefore do not affect blocks in $\mathcal{S}$. That is, the bc‑block‑producer can always just omit non‑coinbase transactions that are contextually invalid.
* The [**Genesis bc‑block rule**](./construction.md#genesis-bc-block-rule) doesn’t apply to new bc‑blocks.
* The [**Valid context rule**](./construction.md#valid-context-rule), [**Extension rule**](./construction.md#extension-rule), and [**Last Final Snapshot rule**](./construction.md#last-final-snapshot-rule) are always satisfiable by referencing the same context bft‑block as the parent bc‑block.
* The [**Finality depth rule**](./construction.md#finality-depth-rule) always allows the option of producing a stalled block, and therefore does not affect blocks in $\mathcal{S}$.

The instructions to an honest bc‑block‑producer allow it to produce a block in $\mathcal{S}$. Therefore, $\Pi_{\bc}$ remains live under the same conditions as $\Pi_{\origbc}$.

The additional bft‑proposal‑validity, bft‑block‑validity, bft‑finality, and honest voting rules are also not an obstacle to making, voting for, or finalizing bft‑proposals:

* Because $\Pi_{\bc}$ is live, there will always be some point in time at which a fresh valid bc‑header chain that satisfies both the [**Linearity rule**](./construction.md#linearity-rule) and the [**Tail confirmation rule**](./construction.md#tail-confirmation-rule) exists for use in the $P.\headersbc$ field.
* If no fresh valid bc‑header chain is available, the [**Linearity rule**](./construction.md#linearity-rule) and [**Tail confirmation rule**](./construction.md#tail-confirmation-rule) allow an honest bft‑proposer to choose $\headersbc$ to be the same as in the previous bft‑block. So, if liveness of $\Pi_{\origbft}$ depends on an honest proposer always being able to make a proposal (as it does in adapted‑Streamlet for example), then this requirement will not be violated.
* The changes to voting are only requiring a vote to be for a proposal that could have been honestly proposed.
* The bft‑finality rules are unchanged from origbft‑finality.

Therefore, $\Pi_{\bft}$ remains live under the same conditions as $\Pi_{\origbft}$.

The only other possibility for a liveness issue in Crosslink 2 would be if the change to the constructions of $\localfin_i$ or $(\localba_\mu)_i$ could cause either of them to stall, even when $\Pi_{\bft}$ and $\Pi_{\bc}$ are both still live.

However, liveness of $\Pi_{\bft}$ and the [**Linearity rule**](./construction.md#linearity-rule) together imply that at each point in time, provided there are sufficient honest bft‑proposers/validators, eventually a new bft‑block with a higher-scoring snapshot will become final in the context of the longest bft‑valid‑chain. $\TODO$ make that more precise.

Because of the [**Extension rule**](./construction.md#extension-rule), this new bft‑block must be a descendent of the previous final bft‑block in the context visible to bc‑block‑producers. Therefore, the new finalized chain will extend the old finalized chain.

Finally, we need to show that Stalled Mode is only triggered when it should be; that is, when the assumptions needed for liveness of $\Pi_{\bft}$ are violated. Informally, that is the case because, as long as there are sufficient honest bc‑block‑producers *and* sufficient honest bft‑proposers/validators, the finalization point implied by the $\contextbft$ field at the tip of the bc‑best chain in any node’s view will advance fast enough for the finalization gap bound $L$ not to be hit. This depends on the value of $L$ relative to $\sigma$, the network delay, the hash rate of honest bc‑block‑producers, the number of honest bft‑proposers and the proportion of voting units they hold, and other details of the BFT protocol. $\TODO$ more detailed argument needed, especially for the dependence on $L$.

## Safety argument

$\TODO$ Not updated for Crosslink 2 below this point.

### Discussion

Recall the definition of [**Assured Finality**](./construction.md#assured-finality).

<span id="assured-finality"></span>
```admonish success "Definition: Assured Finality"
An execution of Crosslink 2 has **Assured Finality** iff <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have <span style="white-space: nowrap">$\localfin_i^t \agrees_{\bc} \localfin_j^u$.</span>
```

First we prove that **Assured Finality** is implied by [**Prefix Agreement**](./construction.md#prefix-agreement) of $\Pi_{\bc}$.

<span id="prefix-agreement"></span>
```admonish success "Definition: Prefix Agreement"
An execution of $\Pi_{\starbc}$ has **Prefix Agreement** at confirmation <span style="white-space: nowrap">depth $\sigma$,</span> iff <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have <span style="white-space: nowrap">$\ch_i^t \trunc_{\!\starbc}^\sigma\, \agrees_{\starbc} \ch_j^u \trunc_{\!\starbc}^\sigma$.</span>
```

<span id="prefix-agreement-implies-assured-finality"></span>
```admonish success "Safety Theorem: Prefix Agreement of Π<sub>bc</sub> implies Assured Finality"
In an execution of Crosslink 2 for which the $\Pi_{\bc}$ subprotocol has **Prefix Agreement** at confirmation <span style="white-space: nowrap">depth $\sigma$,</span> that execution has **Assured Finality**.

Proof: Suppose that we have times <span style="white-space: nowrap">$t$, $u$</span> and <span style="white-space: nowrap">nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$.</span> Then, by the [**Local fin-depth lemma**](./construction.md#local-fin-depth) applied to each of <span style="white-space: nowrap">node $i$</span> and <span style="white-space: nowrap">node $j$,</span> there exist <span style="white-space: nowrap">times $r$</span> at which <span style="white-space: nowrap">node $i$</span> is honest and <span style="white-space: nowrap">$r'$</span> at which <span style="white-space: nowrap">node $j$</span> is honest, such that $\localfin_i^t \preceq_{\bc} \ch_i^r \trunc_{\bc}^\sigma$ and <span style="white-space: nowrap">$\localfin_j^u \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> By [**Prefix Agreement at confirmation depth $\boldsymbol{\sigma}$**](./construction.md#prefix-agreement), we have <span style="white-space: nowrap">$\ch_i^r \trunc_{\bc}^\sigma\, \agrees_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> Wlog due to symmetry, suppose <span style="white-space: nowrap">$\ch_i^r \trunc_{\bc}^\sigma\, \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> Then $\localfin_i^t \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$</span> (by transitivity <span style="white-space: nowrap">of $\preceq_{\bc}$)</span> and $\localfin_j^u \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$ (as above),</span> so <span style="white-space: nowrap">$\localfin_i^t \agrees_{\bc} \localfin_j^u$</span> by the [**Linear prefix lemma**](./construction.md#linear-prefix-lemma).
```

Then we prove that **Assured Finality** is also implied by [**Final Agreement**](./construction.md#final-agreement) of $\Pi_{\bft}$.

<span id="final-agreement"></span>
```admonish success "Definition: Final Agreement"
An execution of $\Pi_{\starbft}$ has **Final Agreement** iff for all <span style="white-space: nowrap">$\star$bft‑valid blocks $C$</span> in honest view at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$C'$ in honest view</span> at <span style="white-space: nowrap">time $t'$,</span> we have <span style="white-space: nowrap">$\star\bftlastfinal(C) \agrees_{\starbft} \star\bftlastfinal(C')$.</span>
```

<span id="final-agreement-implies-assured-finality"></span>
```admonish success "Safety Theorem: Final Agreement of Π<sub>bft</sub> implies Assured Finality"
In an execution of Crosslink 2 for which the $\Pi_{\bft}$ subprotocol has **Final Agreement**, that execution has **Assured Finality**.

Proof: Suppose that we have times <span style="white-space: nowrap">$t$, $u$</span> and <span style="white-space: nowrap">nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$.</span> Then, by the [**Local fin-depth lemma**](./construction.md#local-fin-depth) applied to each of <span style="white-space: nowrap">node $i$</span> and <span style="white-space: nowrap">node $j$,</span> there exist <span style="white-space: nowrap">times $r$</span> at which <span style="white-space: nowrap">node $i$</span> is honest and <span style="white-space: nowrap">$r'$</span> at which <span style="white-space: nowrap">node $j$</span> is honest, such that $\localfin_i^t \preceq_{\bc} \ch_i^r \trunc_{\bc}^\sigma$ and <span style="white-space: nowrap">$\localfin_j^u \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> By [**Prefix Agreement at confirmation depth $\boldsymbol{\sigma}$**](./construction.md#prefix-agreement), we have <span style="white-space: nowrap">$\ch_i^r \trunc_{\bc}^\sigma\, \agrees_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> Wlog due to symmetry, suppose <span style="white-space: nowrap">$\ch_i^r \trunc_{\bc}^\sigma\, \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$.</span> Then $\localfin_i^t \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$</span> (by transitivity <span style="white-space: nowrap">of $\preceq_{\bc}$)</span> and $\localfin_j^u \preceq_{\bc} \ch_j^{r'} \trunc_{\bc}^\sigma$ (as above),</span> so <span style="white-space: nowrap">$\localfin_i^t \agrees_{\bc} \localfin_j^u$</span> by the [**Linear prefix lemma**](./construction.md#linear-prefix-lemma).
```

<span id="prefix-consistency"></span>
```admonish success "Safety Theorem: Prefix Consistency of Π<sub>bc</sub> implies Prefix Consistency of ba"
By the [**Local ba-depth lemma**](./construction.md#local-ba-depth), we have:

> In any execution of Crosslink 2, for any <span style="white-space: nowrap">confirmation depth $\mu \leq \sigma$</span> and any <span style="white-space: nowrap">node $i$</span> that is honest at <span style="white-space: nowrap">time $t$,</span> there exists a <span style="white-space: nowrap">time $r \leq t$</span> <span style="white-space: nowrap">such that $(\localba_\mu)_i^t \preceq_{\bc} \ch_i^r \trunc_{\bc}^\mu$.</span>

Renaming $t$ to $r$ and $\sigma$ to $\mu$ in the definition of [**Prefix Consistency**](./construction.md#prefix-consistency) gives:

> An execution of $\Pi_{\bc}$ has **Prefix Consistency** at confirmation depth $\mu$, iff <span style="white-space: nowrap">for all times $r \leq u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $r$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have that <span style="white-space: nowrap">$\ch_i^r \trunc_{\bc}^\mu\, \preceq_{\bc} \ch_j^u$.</span>

Since any node $i$ that is honest at time $t$ is also honest at time $r \leq t$, and by transitivity of $\preceq_{\bc}$, we therefore have:

> In any execution of Crosslink 2 that has **Prefix Consistency** at confirmation depth $\mu$, <span style="white-space: nowrap">for all times $t \leq u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have that <span style="white-space: nowrap">$(\localba_\mu)_i^t \preceq_{\bc} \ch_j^u$.</span>
```
----

The **Extension rule** ensures that, informally, if a given node $i$’s view of its bc‑best‑chain at a depth of $\sigma$ blocks does not roll back, then neither does its view of the bft‑final block referenced by its bc‑best‑chain, and therefore neither does its view of $\LOG_{\fin,i}^t$.

This does *not* by itself imply that all nodes are seeing the “same” confirmed bc‑best‑chain (up to propagation timing), or the same $\LOG_{\fin,i}^t$. If the network is partitioned *and* $\Pi_{\bft}$ is subverted, it could be that the nodes on each side of the partition follow a different fork, and the adversary arranges for each node’s view of the last final bft‑block to be consistent with the fork it is on. It can potentially do this if it has more than one third of validators, because if the validators are partitioned in the same way as other nodes, it can vote with an additional one third of them on each side of the fork.

This is, if you think about it, unavoidable. $\Pi_{\bc}$ doesn’t include the mechanisms needed to maintain finality under partition; it takes a different position on the CAP trilemma. In order to maintain finality under partition, we need $\Pi_{\bft}$ not to be subverted (and to actually work!)

So what is the strongest security property we can realistically get? It is stronger than what Snap‑and‑Chat provides. Snap‑and‑Chat is unsafe *even without a partition* if $\Pi_{\bft}$ is subverted. Ideally we would have a protocol with safety that is *only* limited by attacks “like” the unavoidable attack described above — which also applies to $\Pi_{\bc}$ used on its own.

### Proof of safety for LOG<sub>fin</sub>

In order to capture the intuition that it is hard in practice to cause a consistent partition of the kind described in the previous section, we will need to assume that the [**Prefix Agreement**](./construction.md#prefix-agreement) safety property holds for the relevant executions of $\Pi_{\bc}$. The structural and consensus modifications to $\Pi_{\bc}$ relative to $\Pi_{\origbc}$ seem unlikely to have any significant effect on this property, given that we proved above that they do not affect liveness. ==TODO: that is a handwave; we should be able to do better, as we do for $\Pi_{\bft}$ below.== So, to the extent that it is reasonable to assume that [**Prefix Agreement**](./construction.md#prefix-agreement) holds for executions of $\Pi_{\origbc}$ under some conditions, it should also be reasonable to assume it holds for executions of $\Pi_{\bc}$ under the same conditions.

Recall that $\LF(H) := \bftlastfinal(H\dot\contextbft)$.

<span id="prefix-lemma"></span>
```admonish success "Prefix Lemma"
If $H_1$, $H_2$ are bc‑valid blocks with $H_1 \preceq_{\bc} H_2$, then $\LF(H_1) \preceq_{\bft} \LF(H_2)$.

Proof: Using the **Extension rule**, by induction on the distance between $H_1$ and $H_2$.
```

Using the **Prefix Lemma** once for each direction, we can transfer the [**Prefix Agreement**](./construction.md#prefix-agreement) property to the referenced bft‑blocks:

<span id="prefix-agreement-lemma"></span>
```admonish success "Prefix Agreement Lemma"
In an execution of $\Pi_{\bc}$ that has [**Prefix Agreement**](./construction.md#prefix-agreement) at confirmation <span style="white-space: nowrap">depth $\sigma$,</span> <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> we have <span style="white-space: nowrap">$\LF(\ch_i^t \trunc_{\bc}^\sigma) \agrees_{\bft} \LF(\ch_j^u \trunc_{\bc}^\sigma)$.</span>
```

Let $\textsf{chain-txns}(\ch)$ be the sequence of transactions in the given <span style="white-space: nowrap">chain $\ch$,</span> starting from genesis.

Recall that $$
\begin{array}{rcl}
\textsf{san-ctx} &\typecolon& [\bcchain] \to \textsf{bc-context} \\
\textsf{san-ctx}(S) &:=& \mathsf{sanitize}(\mathsf{concat}([\textsf{chain-txns}(C) \text{ for } C \text{ in } S])) \\
\textsf{fin} &\typecolon& \bcblock \to [\bcchain] \\
\fin(H) &:=& [\snapshot(B) \text{ for } B \preceq_{\bft} \LF(H \trunc_{\bc}^\sigma)] \\
\textsf{fin-ctx}(H) &:=& \textsf{san-ctx}(\fin(H)) \\
\textsf{ba-ctx}(H, \mu) &:=& \textsf{san-ctx}(\fin(H) \,||\, [H \trunc_{\bc}^\mu]) \\
\LOG_{\fin,i}^t &:=& \textsf{context-txns}(\textsf{fin-ctx}(\ch_i^t)) \\
\LOG_{\ba,\mu,i}^t &:=& \textsf{context-txns}(\textsf{ba-ctx}(\ch_i^t, \mu))
\end{array}
$$

Because $\fin$ takes the form <span style="white-space: nowrap">$\fin(H) := [f(X) \text{ for } X \preceq_{\bft} g(H)]$,</span> we have that <span style="white-space: nowrap">$g(H) \preceq_{\bft} g(H') \implies \fin(H) \preceq \fin(H')$.</span> (This would be true for any well‑typed <span style="white-space: nowrap">$f$ and $g$.)</span>

By this observation and the **Prefix Agreement Lemma**, we also have that, in an execution of Crosslink 2 where $\Pi_{\bc}$ has the [**Prefix Agreement**](./construction.md#prefix-agreement) safety property at confirmation depth <span style="white-space: nowrap">$\sigma$,</span> <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> <span style="white-space: nowrap">$\fin(\ch_i^t) \agrees \fin(\ch_j^u)$.</span>

Because $\mathsf{sanitize}$ only considers previous state, $\textsf{context-txns}$ ∘ $\textsf{san-ctx}$ must be a prefix-preserving map; that is, <span style="white-space: nowrap">if $S_1 \preceq S_2$ then $\textsf{context-txns}(\textsf{san-ctx}(S_1)) \preceq \textsf{context-txns}(\textsf{san-ctx}(S_2))$.</span> Therefore:

```admonish success "Theorem: LOG<sub>fin</sub> Safety (from Prefix Agreement of Π<sub>bc</sub>)"
In an execution of Crosslink 2 where $\Pi_{\bc}$ has [**Prefix Agreement**](./construction.md#prefix-agreement) at confirmation depth $\sigma$, <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> <span style="white-space: nowrap">$\LOG_{\fin,i}^t \agrees \LOG_{\fin,j}^u\,$.</span>
```

Notice that this does not depend on any safety property of $\Pi_{\bft}$, and is an elementary proof. ([[NTT2020](https://eprint.iacr.org/2020/1091.pdf), Theorem 2] is a much more complicated proof that takes nearly 3 pages, not counting the reliance on results from [[PS2017]](https://eprint.iacr.org/2016/918.pdf).)

*In addition,* just as in Snap‑and‑Chat, safety of $\LOG_{\fin}$ can be inferred from safety of <span style="white-space: nowrap">$\Pi_{\bft}$,</span> which follows from safety of <span style="white-space: nowrap">$\Pi_{\origbft}$.</span> We prove this based on the [**Final Agreement**](./construction.md#final-agreement) property for executions of <span style="white-space: nowrap">$\Pi_{\origbft}$:</span>

```admonish success "Definition: Final Agreement"
An execution of $\Pi_{\origbft}$ has the [**Final Agreement**](./construction.md#final-agreement) safety property iff for all origbft‑valid <span style="white-space: nowrap">blocks $C$</span> [in honest view](#in-honest-view) at <span style="white-space: nowrap">time $t$</span> and $C'$ in honest view at <span style="white-space: nowrap">time $t'$,</span> we have <span style="white-space: nowrap">$\origbftlastfinal(C) \agrees_{\origbft} \origbftlastfinal(C')\,$.</span>
```

The changes in $\Pi_{\bft}$ relative to $\Pi_{\origbft}$ only add structural components and tighten bft‑block‑validity and bft‑proposal‑validity rules. So for any legal execution of $\Pi_{\bft}$ there is a corresponding legal execution of $\Pi_{\origbft}$, with the structural additions erased and with the same nodes honest at any given time. A safety property, by definition, only asserts that executions not satisfying the property do not occur. Safety properties of $\Pi_{\origbft}$ necessarily do not refer to the added structural components in $\Pi_{\bft}$. Therefore, for any safety property of $\Pi_{\origbft}$, including [**Final Agreement**](./construction.md#final-agreement), the corresponding safety property holds for $\Pi_{\bft}$.

By the definition of $\fin$ as above, in an execution of Crosslink 2 where $\Pi_{\bft}$ has [**Final Agreement**](./construction.md#final-agreement), <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> <span style="white-space: nowrap">$\fin(\ch_i^t) \agrees \fin(\ch_j^u)\,$.</span> Therefore, by an argument similar to the one above using the fact that $\textsf{context-txns}$ ∘ $\textsf{san-ctx}$ is a prefix-preserving map:

```admonish success "Theorem: LOG<sub>fin</sub> Safety (from Final Agreement of Π<sub>bft</sub> or Π<sub>origbft</sub>)"
In an execution of Crosslink 2 where $\Pi_{\bft}$ has [**Final Agreement**](./construction.md#final-agreement), <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> <span style="white-space: nowrap">$\LOG_{\fin,i}^t \agrees \LOG_{\fin,j}^u\,$.</span>
```

### Proof of safety for LOG<sub>ba</sub>

From the two $\LOG_{\fin}$ Safety theorems and the [**Ledger prefix property**](./construction.md#admonition-theorem-ledger-prefix-property), we immediately have:

```admonish success "Theorem: LOG<sub>ba</sub> does not roll back past the agreed LOG<sub>fin</sub>"
Let $\mu_i$ be an arbitrary choice of $\LOG_{\ba}$ confirmation depth for each node $i$. Consider an execution of Crosslink 2 where either $\Pi_{\bc}$ has [**Prefix Agreement**](./construction.md#prefix-agreement) at confirmation depth $\sigma$ or $\Pi_{\bft}$ has [**Final Agreement**](./construction.md#final-agreement).

In such an execution, <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> either $\LOG_{\fin,i}^t \preceq_{\bc} \LOG_{\fin,j}^u \preceq_{\bc} \LOG_{\ba,\mu_j,j}^u$ or $\LOG_{\fin,j}^u \preceq_{\bc} \LOG_{\fin,i}^t \preceq_{\bc} \LOG_{\ba,\mu_i,i}^t$.

**Corollary:** Under the same conditions, if wlog $\LOG_{\fin,i}^t \preceq_{\bc} \LOG_{\fin,j}^u$, then $\LOG_{\fin,i}^t \preceq_{\bc} \{ \LOG_{\ba,\mu_i,i}^t, \LOG_{\ba,\mu_j,j}^u \}$.
```

The above property is not as strong as we would like for practical uses of $\LOG_{\ba}$, because it does not say anything about rollbacks up to the finalization point, and such rollbacks may be of unbounded length. (Loosely speaking, the number of non‑Stalled Mode bc‑blocks after the consensus finalization point is bounded by <span style="white-space: nowrap">$L$,</span> but we have also not proven that so far.)

As documented in the [Model for BFT protocols](./construction.md#model-for-bft-protocols-Πorigbftbft) section of [The Crosslink 2 Construction](./construction.md)):

> For each epoch, there is a fixed number of voting units distributed between the players, which they use to vote for a <span style="white-space: nowrap">$\star$bft‑proposal.</span> We say that a voting unit has been cast for a <span style="white-space: nowrap">$\star$bft‑proposal $P$</span> at a given time in a <span style="white-space: nowrap">$\star$bft‑execution,</span> <span style="white-space: nowrap">if and only if</span> <span style="white-space: nowrap">$P$ is $\star$bft‑proposal‑valid</span> and a ballot <span style="white-space: nowrap">for $P$</span> authenticated by the holder of the voting unit exists at that time.
>
> Using knowledge of ballots cast for a <span style="white-space: nowrap">$\star$bft‑proposal $P$</span> that collectively satisfy a notarization rule at a given time in a <span style="white-space: nowrap">$\star$bft‑execution,</span> and only with such knowledge, it is possible to obtain a valid <span style="white-space: nowrap">$\star$bft‑notarization‑proof $\proof_P$.</span> The notarization rule must require at least a two‑thirds absolute supermajority of voting units <span style="white-space: nowrap">in $P$’s epoch</span> to have been cast <span style="white-space: nowrap">for $P$.</span> It may also require other conditions.
>
> A voting unit is cast non‑honestly for an epoch’s proposal iff:
> * it is cast other than by the holder of the unit (due to key compromise or any flaw in the voting protocol, for example); or
> * it is double‑cast (i.e. there are two ballots casting it for distinct proposals); or
> * the holder of the unit following the conditions for honest voting <span style="white-space: nowrap">in $\Pi_{\starbft}$,</span> according to its view, should not have cast that vote.

<span id="one-third-bound"></span>
```admonish success "Definition: One‑third bound on non‑honest voting"
An execution of $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property iff for every epoch, *strictly* fewer than one third of the total voting units for that epoch are ever cast non‑honestly.
```

<span id="theorem-on-bft-valid-blocks"></span>
```admonish success "Theorem: On bft‑valid blocks for a given epoch in honest view"
By a well known argument often used to prove safety of BFT protocols, in an execution of Crosslink 2 where $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property (and assuming soundness of notarization proofs), any bft‑valid block for a given epoch [in honest view](#in-honest-view) must commit to the same proposal.

Proof (adapted from [[CS2020](https://eprint.iacr.org/2020/088.pdf), Lemma 1]): Suppose there are two bft‑proposals $P$ and $P'$, both for <span style="white-space: nowrap">epoch $e$,</span> such that <span style="white-space: nowrap">$P$ is</span> committed to by some bft‑block‑valid <span style="white-space: nowrap">block $B$,</span> and <span style="white-space: nowrap">$P'$ is</span> committed to by some bft‑block‑valid <span style="white-space: nowrap">block $B'$.</span> This implies that $B$ and $B'$ have valid notarization proofs. Let the number of voting units for <span style="white-space: nowrap">epoch $e$</span> <span style="white-space: nowrap">be $n_e$.</span> Assuming soundness of the notarization proofs, it must be that at least $2n_e/3$ voting units for <span style="white-space: nowrap">epoch $e$,</span> denoted as the <span style="white-space: nowrap">set $S$,</span> were cast <span style="white-space: nowrap">for $P$,</span> and at least $2n_e/3$ voting units for <span style="white-space: nowrap">epoch $e$,</span> denoted as the <span style="white-space: nowrap">set $S'$,</span> were cast <span style="white-space: nowrap">for $P'$.</span> Since there are $n_e$ voting units for <span style="white-space: nowrap">epoch $e$,</span> $S \intersection S'$ must have size at least <span style="white-space: nowrap">$n_e/3$.</span> In an execution of Crosslink 2 where $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property, $S \intersection S'$ must therefore include at least one voting unit that was cast honestly. Since a voting unit for epoch $e$ that is cast honestly is not double-cast, it must be <span style="white-space: nowrap">that $P = P'$.</span>
```

```admonish info
In the case of a network partition, votes may not be seen on both/all sides of the partition. Therefore, it is not necessarily the case that honest nodes can directly see double‑voting. The above argument does not depend on being able to do so.
```

Therefore, in an execution of Crosslink 2 for which $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property, for each <span style="white-space: nowrap">epoch $e$</span> there will be at most one bft‑proposal‑valid <span style="white-space: nowrap">proposal $P_e$,</span> and at least one third of honestly cast voting units must have been cast for it. Let $\mathcal{I}_e$ be the (necessarily nonempty) set of nodes that cast these honest votes; then, $\snapshot(P_e) \preceq_{\bc} \ch_i^{t_{e,i}} \trunc_{\bc}^\sigma$ for <span style="white-space: nowrap">all $i \in \mathcal{I}_e$</span> at the <span style="white-space: nowrap">times $t_{e,i}$</span> of their votes in epoch $e$. <span style="white-space: nowrap">(For simplicity,</span> we assume that for each honest <span style="white-space: nowrap">node $i$</span> there is only one <span style="white-space: nowrap">time $t_{e,i}$</span> at which it obtains a successful check for the voting condition in <span style="white-space: nowrap">epoch $e$,</span> which it uses for any votes that it casts in that epoch.)

Let $B$ be any bft‑block for <span style="white-space: nowrap">epoch $e$</span> such that <span style="white-space: nowrap">$B \preceq_{\bft} \bftlastfinal(C)$,</span> where $C$ is some bft‑block‑valid block. <span style="white-space: nowrap">Since $B \preceq_{\bft} C$,</span> <span style="white-space: nowrap">$B$ is bft‑block‑valid.</span> So by the argument above, $B$ commits to the only bft‑proposal‑valid <span style="white-space: nowrap">proposal $P_e$</span> for <span style="white-space: nowrap">epoch $e$,</span> and $\snapshot(B) = \snapshot(P_e)$ was voted for in that epoch by a nonempty subset of honest <span style="white-space: nowrap">nodes $\mathcal{I}_e$.</span>

Let $H$ be any bc‑valid block. We have by definition: $$
\begin{array}{rcl}
\fin(H) &\!\!=\!\!& [\snapshot(B) \text{ for } B \preceq_{\bft} \LF(H \trunc_{\bc}^\sigma)] \\
&\!\!=\!\!& [\snapshot(B) \text{ for } B \preceq_{\bft} \bftlastfinal(H \trunc_{\bc}^\sigma\dot\contextbft)]
\end{array}
$$ So, taking $C = H \trunc_{\bc}^\sigma\dot\contextbft$, each $\snapshot(B)$ for $B$ of epoch $e$ in the result of $\fin(H)$ satisfies $\snapshot(B) \preceq_{\bc} \ch_i^{t_{e,i}} \trunc_{\bc}^\sigma$ for all $i$ in some nonempty honest set of nodes $\mathcal{I}_e$.

For an execution of Crosslink 2 in which $\Pi_{\bc}$ has the [**Prefix Consistency**](./construction.md#prefix-consistency) property at confirmation depth $\sigma$, for every epoch $e$, for every such $(i, t_{e,i})$, for every node $j$ that is honest at any time $u \geq t_{e,i}$, we have $\ch_i^{t_{e,i}} \trunc_{\bc}^\sigma \preceq_{\bc} \ch_j^u$. Let $t_e = \min \{ t_{e,i} : i \in \mathcal{I}_e \}$. Then, by transitivity of $\preceq_{\bc}$:

```admonish success "Theorem: On snapshots in LOG<sub>fin</sub>"
In an execution of Crosslink 2 where $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property and $\Pi_{\bc}$ has the [**Prefix Consistency**](./construction.md#prefix-consistency) property at confirmation <span style="white-space: nowrap">depth $\sigma$,</span> every bc‑chain $\snapshot(B)$ in $\fin(\ch_i^t)$ (and therefore every snapshot that contributes to <span style="white-space: nowrap">$\LOG_{\fin,i}^t$)</span> is, <span style="white-space: nowrap">at any time $u \geq t_e$</span>,</span> in the bc‑best‑chain of <span style="white-space: nowrap">every node $j$</span> that is honest <span style="white-space: nowrap">at time $u$</span> (where $B$ commits to $P_e$ at epoch $e$ and $t_e$ is the time of the first honest vote <span style="white-space: nowrap">for $P_e$).</span>
```

A similar (weaker) statement holds if we replace $u \geq t_e$ with $u \geq t$, since the time of the first honest vote for $P$ necessarily precedes the time at which the signed $(P, \proof_P)$ is submitted as a bft‑block, which necessarily <span style="white-space: nowrap">precedes $t$.</span> (Whether or not the notarization proof depends on the *first* honest vote for <span style="white-space: nowrap">$B$’s proposal $P_e$,</span> it must depend on some honest vote for that proposal that was not made earlier <span style="white-space: nowrap">than $t_e$.)</span>

Furthermore, we have $$
\begin{array}{rcl}
\textsf{ba-ctx}(H, \mu) &\!\!\!=\!\!\!& \textsf{san-ctx}(\fin(H) \,||\, [H \trunc_{\bc}^\mu]) \\
\LOG_{\ba,\mu,i}^t &\!\!\!=\!\!\!& \textsf{context-txns}(\textsf{ba-ctx}(\ch_i^t, \mu))
\end{array}
$$

So in an execution of Crosslink 2 where $\Pi_{\bc}$ has the [**Prefix Consistency**](./construction.md#prefix-consistency) property at confirmation <span style="white-space: nowrap">depth $\mu$,</span> <span style="white-space: nowrap">if node $i$</span> is honest <span style="white-space: nowrap">at time $t$</span> then $H \trunc_{\bc}^\mu$ is also, <span style="white-space: nowrap">at any time $u \geq t$,</span> in the bc‑best‑chain of <span style="white-space: nowrap">every node $j$</span> that is honest <span style="white-space: nowrap">at time $u$.</span>

If an execution of $\Pi_{\bc}$ has the [**Prefix Consistency**](./construction.md#prefix-consistency) property at confirmation <span style="white-space: nowrap">depth $\mu \leq \sigma$,</span> then it necessarily also has it at confirmation <span style="white-space: nowrap">depth $\sigma$.</span> Therefore we have:

```admonish success "Theorem: On snapshots in LOG<sub>ba</sub>"
In an execution of Crosslink 2 where $\Pi_{\bft}$ has the **one‑third bound on non‑honest voting** property and $\Pi_{\bc}$ has the [**Prefix Consistency**](./construction.md#prefix-consistency) property at confirmation <span style="white-space: nowrap">depth $\mu \leq \sigma$,</span> every bc‑chain snapshot in $\fin(\ch_i^t) \,||\, [\ch_i^t \trunc_{\bc}^\mu]$ (and therefore every snapshot that contributes to <span style="white-space: nowrap">$\LOG_{\ba,\mu,i}^t$)</span> is, <span style="white-space: nowrap">at any time $u \geq t$,</span> in the bc‑best‑chain of <span style="white-space: nowrap">every node $j$</span> that is honest <span style="white-space: nowrap">at time $u$.</span>
```

Sketch: we also need the sequence of snapshots output from fin to only be extended in the view of any node. In that case we can infer that the node does not observe a rollback in LOG_ba.

Recall that in the proof of safety for $\LOG_{\fin}$, we showed that in an execution of Crosslink 2 where $\Pi_{\bft}$ (or $\Pi_{\origbft}$) has [**Final Agreement**](./construction.md#final-agreement), <span style="white-space: nowrap">for all times $t$, $u$</span> and <span style="white-space: nowrap">all nodes $i$, $j$</span> (potentially the same) such that <span style="white-space: nowrap">$i$ is honest</span> at <span style="white-space: nowrap">time $t$</span> and <span style="white-space: nowrap">$j$ is honest</span> at <span style="white-space: nowrap">time $u$,</span> <span style="white-space: nowrap">$\fin(\ch_i^t) \agrees \fin(\ch_j^u)\,$.</span>

What we want to show is that, under some conditions on executions, ...

## Disadvantages of Crosslink

### More invasive changes

Unlike Snap‑and‑Chat, Crosslink 2 requires structural and consensus rule changes to both $\Pi_{\bc}$ and $\Pi_{\bft}$. On the other hand, several of those changes are arguably necessary to fix a show‑stopping bug in Snap‑and‑Chat (not being able to spend some finalized outputs).

### Finalization latency

For a given choice of $\sigma$, the finalization latency is higher. The snapshot of the BFT chain used to obtain $\LOG_{\fin,\mu,i}^t$ is obtained from the block at depth $\mu$ on node $i$’s best $\Pi_{\bc}$ chain, which will on average lead to a finalized view that is about $\mu + 1 + \sigma$ blocks back (in $\Pi_{\bc}$), rather than $\sigma_{\sac}}$ blocks in Snap‑and‑Chat. This is essentially the cost of ensuring that safety is given by the stronger of the safety of $\Pi_{\bc}$ (at $\mu$ confirmations) and the safety of $\Pi_{\bft}$.

On the other hand, the relative increase in expected finalization latency is only $\frac{\mu + 1 + \sigma}{\sigma_{\sac}}},$ i.e. at most slightly more than a factor of 2 for the case $\mu = \sigma = \sigma_{\sac}}$.

### More involved liveness argument

See the Liveness section above.

## Every rule in Crosslink 2 is individually necessary

```admonish warning
In order to show that Crosslink 2 is at a *local* optimum in the security/complexity trade‑off space, for each rule we show attacks on safety and/or liveness that could be performed if that rule were omitted or simplified.
```

Edit: some rules, e.g. the [**Linearity rule**](./construction.md#linearity-rule), only contribute heuristically to security in the analysis so far.
