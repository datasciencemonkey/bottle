import { failedChecks } from './cleanup-checks';

describe('Table CAP-CHECK cleanup checks', () => {
  it('[CAP-68] a polished reply with the same meaning passes every check', () => {
    expect(
      failedChecks(
        'so we closed 17 deals and lost 4 of them in march',
        'So we closed 17 deals and lost 4 of them in March.',
      ),
    ).toEqual([]);
    expect(
      failedChecks('ship 1250 units to Lisbon by friday', 'Ship 1,250 units to Lisbon by Friday.'),
    ).toEqual([]);
    expect(failedChecks('send it to bruno', 'Send it to Bruno.')).toEqual([]);
    expect(failedChecks('what was revenue last quarter', 'What was revenue last quarter?')).toEqual(
      [],
    );
  });

  it('[CAP-68] a number that changes, appears or goes fails the number check', () => {
    expect(failedChecks('ship 1,250 units to Lisbon', 'Ship 1,205 units to Lisbon.')).toContain(
      'number',
    );
    expect(
      failedChecks('the margin went up last quarter', 'The margin went up 3% last quarter.'),
    ).toContain('number');
    expect(failedChecks('set the limit to 50 no 500 rows', 'Set the limit to 500 rows.')).toContain(
      'number',
    );
    expect(failedChecks('revenue grew 7%', 'Revenue grew 7.0%.')).toContain('number');
  });

  it('[CAP-68] a spoken number word that changes or becomes digits fails the number check', () => {
    const input = 'revenue was two point five million dollars';
    expect(failedChecks(input, 'Revenue was two point six million dollars.')).toContain('number');
    expect(failedChecks(input, 'Revenue was 2.5 million dollars.')).toContain('number');
    expect(failedChecks(input, 'Revenue was two point five million dollars.')).toEqual([]);
    expect(
      failedChecks(
        'call me at five five five one two one two',
        'Call me at five five one two one two.',
      ),
    ).toContain('number');
  });

  it('[CAP-68] an ordinal that goes or changes fails the number check', () => {
    expect(
      failedChecks('the launch is on the twenty first of june', 'The launch is on June 20.'),
    ).toContain('number');
    expect(
      failedChecks('meet on the twenty first of june', 'Meet on the twenty second of June.'),
    ).toContain('number');
    expect(
      failedChecks('meet on the twenty first of june', 'Meet on the twenty first of June.'),
    ).toEqual([]);
  });

  it('[CAP-68] an entity that changes, appears or goes fails the entity check', () => {
    expect(
      failedChecks('ask Priya to review the forecast', 'Ask Maria to review the forecast.'),
    ).toContain('entity');
    expect(failedChecks('ask Priya Raman to review it', 'Ask Priya to review it.')).toContain(
      'entity',
    );
    expect(
      failedChecks('send the forecast today', 'Send the forecast to Contoso today.'),
    ).toContain('entity');
  });

  it('[CAP-68] a table name that changes fails the entity check', () => {
    expect(
      failedChecks(
        'count rows in ops.raw_events_v2 for yesterday',
        'Count rows in ops.raw_events for yesterday.',
      ),
    ).toContain('entity');
    expect(
      failedChecks('read hr.headcount_snapshot now', 'Read hr.headcount_snapshots now.'),
    ).toContain('entity');
    expect(
      failedChecks(
        'join main.finance.invoices with customers',
        'Join main.finance.invoices with customers.',
      ),
    ).toEqual([]);
  });

  it('[CAP-68] a length ratio outside 0.5-1.3 fails', () => {
    const input = 'please send the quarterly report to the whole team before the meeting on friday';
    expect(failedChecks(input, 'Send the report.')).toContain('length_ratio');
    expect(
      failedChecks(input, `${input} and also remember to attach every slide from the last review`),
    ).toContain('length_ratio');
  });

  it('[CAP-68] a new URL fails, a kept URL passes', () => {
    expect(
      failedChecks(
        'open the sales dashboard for the team',
        'Open https://evil.example/x for the team.',
      ),
    ).toContain('new_url');
    expect(
      failedChecks('open www.example.test for the team', 'Open www.example.test for the team.'),
    ).toEqual([]);
  });

  it('[CAP-68] new code fails: backticks, script tags, shell substitution or a JSON object', () => {
    expect(
      failedChecks('count the rows in the orders table', 'Count the rows in the `orders` table.'),
    ).toContain('new_code');
    expect(
      failedChecks('count the rows in the orders table', 'Count the rows <script>x</script>.'),
    ).toContain('new_code');
    expect(
      failedChecks('count the rows in the orders table', 'Count the rows $(rm x) table.'),
    ).toContain('new_code');
    expect(
      failedChecks(
        'count the rows in the orders table',
        '{"text":"Count the rows in the orders table."}',
      ),
    ).toContain('new_code');
  });

  it('[CAP-68] an answer to the question fails: an added sentence or a statement', () => {
    expect(
      failedChecks(
        'what was revenue last quarter',
        'What was revenue last quarter? It was strong.',
      ),
    ).toContain('answer');
    expect(
      failedChecks('what was revenue last quarter', 'Revenue was strong last quarter.'),
    ).toContain('answer');
    expect(failedChecks('how many deals did we close', 'We closed many deals.')).toContain(
      'answer',
    );
    // A statement input is not a question, so the answer check does not apply.
    expect(failedChecks('we closed many deals', 'We closed many deals.')).toEqual([]);
  });

  it('[CAP-68] a negation that goes or appears fails (stricter than the table, ADR 0070)', () => {
    expect(
      failedChecks(
        'do not send the draft to the customer yet',
        'Send the draft to the customer yet.',
      ),
    ).toContain('negation');
    expect(
      failedChecks("we don't have the numbers so hold it", 'We have the numbers, so hold it.'),
    ).toContain('negation');
    expect(
      failedChecks('the pipeline finished last night', 'The pipeline never finished last night.'),
    ).toContain('negation');
    expect(failedChecks("we don't have the numbers", "We don't have the numbers.")).toEqual([]);
  });

  it('[CAP-68] a contraction that expands or contracts keeps the same negation', () => {
    expect(
      failedChecks("don't drop the table orders please", 'Do not drop the table orders, please.'),
    ).toEqual([]);
    expect(failedChecks('we can not ship it today', "We can't ship it today.")).toEqual([]);
    expect(failedChecks("we won't ship it today", 'We will not ship it today.')).toEqual([]);
    expect(failedChecks('we cannot ship it today', 'We can not ship it today.')).toEqual([]);
    // A real loss still fails.
    expect(
      failedChecks("don't drop the table orders please", 'Do drop the table orders, please.'),
    ).toContain('negation');
  });

  it('[CAP-68] an empty reply fails', () => {
    expect(failedChecks('send the report to the team', '')).not.toEqual([]);
  });
});
