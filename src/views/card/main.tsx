/** V3 card view entry. The MOG ID comes from the `card-<id>` window label. */
import { mount } from '../mount';
import { ownMogId } from '../view-ipc';
import { CardView } from './CardView';

const mogId = ownMogId('card');
mount(mogId ? <CardView mogId={mogId} /> : <p role="alert">Unknown card window</p>);
