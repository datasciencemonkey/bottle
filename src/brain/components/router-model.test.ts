import {
  GPT_OSS_20B,
  LLAMA_3_1_8B,
  PENDING_ROUTER_EVAL,
  routerModelFor,
  servesLlama318b,
} from './router-model';

const PASSED = { [LLAMA_3_1_8B]: true } as const;

describe('routerModelFor', () => {
  it('[RTR-10a] US regions use Llama 3.1 8B when the strict-schema eval passed', () => {
    for (const region of ['us-east-1', 'us-east-2', 'us-west-2', 'US-East-1 ', 'eastus2'])
      expect(routerModelFor(region, PASSED)).toBe(LLAMA_3_1_8B);
    expect(LLAMA_3_1_8B).toBe('system.ai.meta-llama-3-1-8b-instruct');
  });

  it('[RTR-10a] a US region uses gpt-oss-20b while the SP-13 eval has not passed', () => {
    expect(routerModelFor('us-east-1', PENDING_ROUTER_EVAL)).toBe(GPT_OSS_20B);
    expect(routerModelFor('us-east-1', { [LLAMA_3_1_8B]: false })).toBe(GPT_OSS_20B);
  });

  it('[RTR-10b] EU AWS regions use gpt-oss-20b', () => {
    for (const region of ['eu-central-1', 'eu-west-1'])
      expect(routerModelFor(region, PASSED)).toBe(GPT_OSS_20B);
    expect(GPT_OSS_20B).toBe('system.ai.gpt-oss-20b');
  });

  it('[RTR-10b] other and unknown regions use gpt-oss-20b', () => {
    for (const region of ['ap-southeast-2', 'westeurope', 'australia-east', '', 'usa-fake'])
      expect(routerModelFor(region, PASSED)).toBe(GPT_OSS_20B);
    expect(servesLlama318b('eu-central-1')).toBe(false);
    expect(servesLlama318b('us-west-2')).toBe(true);
  });
});
