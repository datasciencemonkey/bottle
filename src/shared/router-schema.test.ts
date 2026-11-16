import type { Intent, RouterDecision } from './interfaces';
import { MOOD_PRIORITY } from './interfaces';
import { routerJsonSchema } from './router-schema';

type Equal<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;

type Schema = (typeof routerJsonSchema)['schema'];
type RequiredKey = Schema['required'][number];
type PropertyKey = keyof Schema['properties'];
type IntentEnum = Schema['properties']['intent']['enum'][number];
type CommandEnum = Schema['properties']['command']['enum'][number];

// Compile-time checks: `pnpm typecheck` fails if the schema and RouterDecision drift.
const requiredIsRouterDecision: Equal<RequiredKey, keyof RouterDecision> = true;
const propertiesIsRouterDecision: Equal<PropertyKey, keyof RouterDecision> = true;
const intentEnumIsIntent: Equal<IntentEnum, Intent> = true;
const commandEnumIsCommand: Equal<CommandEnum, RouterDecision['command']> = true;

describe('router json_schema (Appendix D)', () => {
  it('its required keys equal the keys of RouterDecision', () => {
    expect(requiredIsRouterDecision).toBe(true);
    expect(propertiesIsRouterDecision).toBe(true);
    const { required, properties } = routerJsonSchema.schema;
    expect([...required].sort()).toEqual(Object.keys(properties).sort());
    expect(required).toHaveLength(14);
  });

  it('its intent and command enums equal the TS unions', () => {
    expect(intentEnumIsIntent).toBe(true);
    expect(commandEnumIsCommand).toBe(true);
  });

  it('is flat and strict', () => {
    expect(routerJsonSchema.name).toBe('mog_router_decision');
    expect(routerJsonSchema.strict).toBe(true);
    expect(routerJsonSchema.schema.type).toBe('object');
    expect(routerJsonSchema.schema.additionalProperties).toBe(false);
  });
});

describe('mood priority (4.8)', () => {
  it('lists the conditions from high to low priority', () => {
    expect(MOOD_PRIORITY).toEqual([
      'paused',
      'capture',
      'error',
      'success',
      'answering',
      'processing',
      'dozing',
      'idle',
    ]);
  });
});
