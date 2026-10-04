# Pactrail v2.1.2

## Conversation workspace

The local web app now follows the supplied DeepSeek Harness screenshot more closely: a workspace sidebar, compact run header, Conversation/Trace navigation, readable task/result thread, bottom composer and slim usage footer. Start with `pactrail web`.

Conversation shows the actual task and stored browser result. Live runs show recorded activity and links to Trace; they never invent model reasoning or tool output. Existing `?view=answer` bookmarks open Conversation. Changes, Evidence, Receipt and guarded Apply/Discard remain available; review remains the default when a candidate awaits a decision.

The bottom input is **Prepare task**, not same-run chat. It saves the brief and opens configuration review for a separate isolated run. A running task still blocks dispatch, and host execution still requires fresh acknowledgement. Its draft persists in this browser.

Single-agent remains the default. Specialist text agents and latent transport remain experimental; the existing diagnostic results are unchanged. No real neural latent communication or efficiency gain is claimed.
