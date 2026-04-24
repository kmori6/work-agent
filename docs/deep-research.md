# Deep Research Processing Flow

Based on **R. Han et al., "Deep Researcher with Test-Time Diffusion"** (arXiv:2507.16075).

Implements the TTD-DR framework: a draft is iteratively refined ("denoised") using retrieved external information via a self-evolutionary algorithm.

Key parameters: `max_revision_steps` = 3, `n_q` = 5, `n_a` = 3.

## Processing Flow

```mermaid
flowchart TD
    A([User Query]) --> B

    subgraph PLAN["1. Research Plan (Self-Evolutionary)"]
        B[Generate Initial Plan] --> C[Critique Plan]
        C --> D[Revise Plan]
    end

    D --> E

    subgraph DRAFT["2. Preliminary Draft"]
        E[Generate Draft\nfrom Plan & Internal Knowledge]
    end

    E --> F

    subgraph LOOP["3. Iterative Denoising Loop  ×max_revision_steps"]
        F[Generate Question Candidates\nn_q = 5] --> G[Select Best Question]
        G --> H[Web Search]
        H --> I[Generate Answer Candidates\nn_a = 3]
        I --> J[Merge Answers]
        J --> K[Denoise Draft\nIntegrate New Q&A]
        K --> L{Coverage\nSufficient?}
        L -- No --> F
        L -- Yes --> M
    end

    M --> N

    subgraph FINAL["4. Final Report (Self-Evolutionary)"]
        N[Generate Initial Report] --> O[Critique Report]
        O --> P[Revise Report]
    end

    P --> Q([Final Report])
```

## Component Details

| Component           | Description                                                     |
| ------------------- | --------------------------------------------------------------- |
| Research Plan       | 3 LLM calls: generate → critique → revise                       |
| Preliminary Draft   | Written from LLM internal knowledge, structured around the plan |
| Question generation | `n_q = 5` candidates targeting draft gaps; best one selected    |
| Answer retrieval    | Web search + `n_a = 3` candidate answers merged into one        |
| Draft denoising     | Full draft revised with new Q&A; reduces gaps                   |
| Exit check          | LLM evaluates plan coverage; exits early if sufficient          |
| Final Report        | Same critique-and-revision loop applied to the completed draft  |
