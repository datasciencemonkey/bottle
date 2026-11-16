/** V1 Hub view (thin). TODO(#226): wizard, settings and history. */
import { mount } from '../mount';

function Hub() {
  return (
    <main>
      <h1>Bottle</h1>
      <p>Databricks Labs / experimental</p>
    </main>
  );
}

mount(<Hub />);
