// The collection dialog (docs/ARCHITECTURE.md §9.6.1, §9.6.2): name, description, one of 48 icons (or the letter
// tile) and one of 12 colours. Creates a collection or edits one. The Access section (D152, CollectionAccess.tsx) is
// shown when creating and to whoever manages the collection: its default access is saved with the rest, its people at
// once.
import { useState, type FormEvent } from 'react';
import { useNavigate } from 'react-router';
import type { Collection } from '../../api/gen/Collection';
import type { CollectionPatch } from '../../api/gen/CollectionPatch';
import type { DefaultAccess } from '../../api/gen/DefaultAccess';
import { useCreateCollection, usePatchCollection } from '../../api/queries/tree';
import { canManage } from '../../lib/access';
import { collectionPath } from '../../lib/urls';
import { closeDialog } from '../../state/dialogs';
import { expandAll } from '../../state/expanded';
import { errorMessage } from '../../state/toasts';
import { CollectionIcon } from '../collection/CollectionIcon';
import { COLLECTION_COLORS, COLLECTION_ICONS, COLLECTION_ICON_NAMES, DEFAULT_COLLECTION_COLOR } from '../collection/icons';
import { Dialog } from '../ui/Dialog';
import { AccessSection } from './CollectionAccess';

export function CollectionDialog({ collection }: { collection: Collection | null }) {
  const navigate = useNavigate();
  const create = useCreateCollection();
  const patch = usePatchCollection();
  const [name, setName] = useState(collection?.name ?? '');
  const [description, setDescription] = useState(collection?.description ?? '');
  const [icon, setIcon] = useState<string | null>(collection?.icon ?? null);
  const [color, setColor] = useState(collection?.color ?? DEFAULT_COLLECTION_COLOR);
  const [defaultAccess, setDefaultAccess] = useState<DefaultAccess>(collection?.default_access ?? 'write');
  const [error, setError] = useState<string | null>(null);
  const busy = create.isPending || patch.isPending;
  const showAccess = collection === null || canManage(collection.my_access);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const n = name.trim();
    if (!n) {
      setError('Give the collection a name.');
      return;
    }
    setError(null);
    try {
      if (collection) {
        const changes: CollectionPatch = { name: n, description, icon, color };
        if (showAccess && defaultAccess !== collection.default_access) changes.default_access = defaultAccess;
        await patch.mutateAsync({ collection, patch: changes });
        closeDialog();
      } else {
        const c = await create.mutateAsync({ name: n, description, icon, color, default_access: defaultAccess });
        closeDialog();
        expandAll([c.id]);
        void navigate(collectionPath(c));
      }
    } catch (err) {
      setError(errorMessage(err, 'The collection was not saved'));
    }
  };

  return (
    <Dialog title={collection ? 'Edit collection' : 'New collection'} onClose={closeDialog} className="collection-dialog">
      <form className="form" onSubmit={(e) => void submit(e)}>
        <div className="collection-name-row">
          <CollectionIcon collection={{ name: name || '?', icon, color }} size={28} />
          <label className="field grow">
            <span>Name</span>
            <input data-autofocus required maxLength={100} value={name} onChange={(e) => setName(e.target.value)} />
          </label>
        </div>
        <label className="field">
          <span>Description</span>
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <fieldset className="field">
          <legend>Colour</legend>
          <div className="swatches" role="radiogroup" aria-label="Colour">
            {COLLECTION_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                role="radio"
                aria-checked={c.toLowerCase() === color.toLowerCase()}
                aria-label={c}
                className="swatch"
                style={{ background: c }}
                onClick={() => setColor(c)}
              />
            ))}
          </div>
        </fieldset>
        <fieldset className="field">
          <legend>Icon</legend>
          <div className="icon-grid" role="radiogroup" aria-label="Icon">
            <button
              type="button"
              role="radio"
              aria-checked={icon === null}
              aria-label="Letter"
              className="icon-choice"
              onClick={() => setIcon(null)}
            >
              <CollectionIcon collection={{ name: name || '?', icon: null, color }} />
            </button>
            {COLLECTION_ICON_NAMES.map((n) => {
              const Icon = COLLECTION_ICONS[n]!;
              return (
                <button
                  key={n}
                  type="button"
                  role="radio"
                  aria-checked={icon === n}
                  aria-label={n}
                  title={n}
                  className="icon-choice"
                  onClick={() => setIcon(n)}
                >
                  <Icon size={16} color={color} aria-hidden />
                </button>
              );
            })}
          </div>
        </fieldset>
        {showAccess && (
          <AccessSection collection={collection} defaultAccess={defaultAccess} onDefaultAccess={setDefaultAccess} />
        )}
        {error && <p className="form-error" role="alert">{error}</p>}
        <div className="form-actions">
          <button type="button" className="button" onClick={closeDialog}>Cancel</button>
          <button type="submit" className="button primary" disabled={busy}>{collection ? 'Save' : 'Create'}</button>
        </div>
      </form>
    </Dialog>
  );
}
