/** V3 card view entry. The MOG ID comes from the `card-<id>` window label. */
import { mount } from '../mount';
import { ownMogId, startCardFocusSignal } from '../view-ipc';
import { CardView } from './CardView';

const mogId = ownMogId('card');
// MB-06a: the card load and focus move the talk-key target (ADR 0058).
if (mogId) startCardFocusSignal(mogId);
mount(mogId ? <CardView mogId={mogId} /> : <p role="alert">Unknown card window</p>);
