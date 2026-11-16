/**
 * The one source of the Table SEC-DISC disclosure copy (SEC-17, AT-24). The hub data panel and
 * the onboarding disclosures render this list; docs/user/where-your-data-goes.md repeats it.
 * `item` is the verbatim item text of Table SEC-DISC in research/final-requirements.md, so the
 * inspection test can match the table. Plain text only: no markdown, no HTML.
 */

export interface Disclosure {
  /** The verbatim item of Table SEC-DISC. */
  readonly item: string;
  /** A stable DOM key (`data-disclosure`). */
  readonly id: string;
  readonly title: string;
  /** True for an item that the table marks "(v1)". */
  readonly v1: boolean;
  /** Plain-text sentences, one paragraph each. */
  readonly lines: readonly string[];
}

export const SEC_DISC: readonly Disclosure[] = [
  {
    item: '"Where your data goes"',
    id: 'where-data-goes',
    title: 'What leaves this computer',
    v1: false,
    lines: [
      'Bottle sends data only to your own Databricks workspaces, and only for the jobs on this page.',
      'Dictation text goes to your home workspace for cleanup, then only to the app you type in.',
      'Nothing goes to Genie until you send it: a question after its preview, or a chip you click.',
      'Your sign-in tokens stay in the Bottle core. They are never written to disk or logs, and the agent never sees them.',
      'Bottle stores your transcripts, answers and memory in an encrypted file on this computer.',
      'Text for cleanup, routing, the memory writer and embeddings goes to your home workspace, as the next sections say.',
      'Telemetry is off unless your admin turns it on.',
      'Databricks logs each model call in system.ai_gateway.usage: who called, from where and how many tokens, but no content.',
      'Databricks keeps model requests in an abuse-check store for up to 30 days.',
      'A model on the Gemini global endpoint can send requests to other regions.',
      'If an admin turns on inference tables, Databricks stores model requests there. They stay off for speech-to-text.',
    ],
  },
  {
    item: 'STT location',
    id: 'stt-location',
    title: 'Where speech-to-text runs',
    v1: false,
    lines: [
      'Speech-to-text runs on this computer by default, with the local Parakeet model.',
      'Push-to-talk audio stays in memory and is never written to disk.',
      'Meeting audio is kept encrypted on disk and deleted within 5 minutes after transcription, unless your admin keeps it longer.',
      'Bottle uses Databricks speech-to-text only if you or your admin turn it on, after a check passes.',
      'Then audio goes to your home workspace in short chunks, and Bottle does not store it there.',
      'The STT badge on each MOG shows which one is on.',
    ],
  },
  {
    item: 'Home-profile processing',
    id: 'home-profile',
    title: 'Your home workspace does the processing',
    v1: false,
    lines: [
      'Each MOG has a home workspace (its home profile).',
      'Text cleanup, the router step, the memory writer, embeddings and the main agent model run there.',
      'Embeddings send the text of memory items to that workspace.',
      'This stays the same when a MOG asks Genie in another workspace.',
      'Questions about another workspace are processed in your home workspace.',
    ],
  },
  {
    item: 'the main agent harness and model location',
    id: 'agent-harness',
    title: 'The main agent and its model',
    v1: false,
    lines: [
      'Each MOG has a main agent with a Claude or Codex harness.',
      'The harness runs on this computer, in one shared runtime with one session for each MOG.',
      'Its model runs through the Databricks AI Gateway in your home workspace with your identity, or as a local model.',
      'The agent gets your cleaned question, the router decision and memory results.',
      'It never gets your sign-in tokens: a local proxy in the Bottle core adds them.',
      'The agent cannot start recording, send context, publish a Page or change the workspace without you.',
      'Dictation does not go through the agent.',
    ],
  },
  {
    item: 'Genie history',
    id: 'genie-history',
    title: 'Genie history',
    v1: false,
    lines: [
      'Genie keeps the questions that Bottle sends in your Genie history in that workspace.',
      'Your admins can also see them in system.access.audit and system.query.history.',
      'You cannot delete these system table rows.',
      'Wipe in Settings can also delete the Genie conversations that Bottle created, where Genie allows it.',
    ],
  },
  {
    item: 'Genie One memory',
    id: 'genie-one-memory',
    title: 'Genie One memory',
    v1: false,
    lines: [
      'Genie One can keep its own memory of your conversations on the Databricks server.',
      'Bottle does not control that memory.',
      'Forget in Bottle gives you a link to review your Genie One memories.',
    ],
  },
  {
    item: "DRAFT UC Pages are visible to Genie One in the owner's conversations",
    id: 'draft-pages',
    title: 'Your DRAFT Pages',
    v1: false,
    lines: [
      'When you approve a definition, Bottle creates a DRAFT Unity Catalog Page that you own, in the home domain of the MOG.',
      'Genie One can read your DRAFT Pages in your own conversations.',
      'Bottle does not publish or delete Pages. You delete them in Unity Catalog.',
    ],
  },
  {
    item: '(v1) the Lakebase PITR window',
    id: 'lakebase-pitr',
    title: 'Lakebase backup window (later version)',
    v1: true,
    lines: [
      'Sync to Lakebase is not in this version. Your memory stays on this computer.',
      'When sync comes, Lakebase keeps a point-in-time backup for 2 days by default.',
      'Deleted data is fully erased only after that window. Bottle will show the value.',
    ],
  },
];
