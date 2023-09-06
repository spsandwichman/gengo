use gix::ThreadSafeRepository;
use crate::{Error, ErrorKind, Result};
use gix::discover::Error as DiscoverError;
use gix::bstr::{BString, ByteSlice};
use std::error::Error as ErrorTrait;
use std::collections::HashMap;
use std::path::Path;
use crate::Language;
use std::sync::atomic::Ordering;
use gix::attrs::StateRef;
use super::Analyzer;

pub struct Handler {
    repository: ThreadSafeRepository,
    rev: String,
}

impl Handler {
    pub fn new<P: AsRef<Path>>(repository: P, rev: &str) -> Result<Self, Box<dyn ErrorTrait>> {
        let repository = match gix::discover(repository) {
            Ok(r) => r,
            Err(DiscoverError::Discover(err)) => {
                return Err(Box::new(Error::with_source(ErrorKind::NoRepository, err)))
            }
            Err(err) => return Err(err.into()),
        };
        let repository = repository.into_sync();

        let rev = rev.to_owned();
        let handler = Self { repository, rev };
        Ok(handler)
    }

    fn analyze<A: Analyzer<P>, P: AsRef<Path>>(&self, analyzer: A) -> crate::Result<()> {
        let local_repo = self.repository.to_thread_local();
        let tree_id = local_repo.rev_parse_single(self.rev.as_str())?.object()?.peel_to_tree()?.id;
        let mut stack = vec![(BString::default(), local_repo, tree_id)];

        let mut all_results = Vec::new();
        while let Some((root, repo, tree_id)) = stack.pop() {
            let is_submodule = !root.is_empty();
            let (state, index) = GitState::new(&repo, &tree_id)?;
            let (mut results, submodule_id_by_path) = Results::from_index(root.clone(), index);

            let submodules = repo.submodules()?.map(|sms| {
                sms.filter_map(|sm| {
                    let path = sm.path().ok()?;
                    let sm_repo = sm.open().ok().flatten()?;
                    Some((path.into_owned(), sm_repo))
                })
                .collect::<HashMap<_, _>>()
            });
            self.analyze_index(analyzer, &repo.into_sync(), &mut results, state, is_submodule)?;
            all_results.push(results);

            if let Some(mut submodules_by_path) = submodules {
                stack.extend(
                    submodule_id_by_path
                        .into_iter()
                        .filter_map(|(path, sm_commit)| {
                            let sm_repo = submodules_by_path.remove(&path)?;
                            let tree_id =
                                sm_repo.find_object(sm_commit).ok()?.peel_to_tree().ok()?.id;
                            let mut abs_root = root.clone();
                            if !abs_root.is_empty() {
                                abs_root.push(b'/');
                            }
                            abs_root.extend_from_slice(&path);
                            Some((abs_root, sm_repo, tree_id))
                        }),
                );
            }
        }

        Ok(())
    }

    fn analyze_index<A: Analyzer<P>, P: AsRef<Path>>(
        &self,
        analyzer: A,
        repo: &gix::ThreadSafeRepository,
        results: &mut Results,
        state: GitState,
        is_submodule: bool,
    ) -> Result<()> {
        gix::parallel::in_parallel_with_slice(
            &mut results.entries,
            None,
            move |_| (state.clone(), repo.to_thread_local()),
            |entry, (state, repo), _, should_interrupt| {
                if should_interrupt.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let Ok(path) =
                    gix::path::try_from_bstr(entry.index_entry.path_in(&results.path_storage))
                else {
                    return Ok(());
                };
                self.analyze_blob(&analyzer, path, repo, state, entry, is_submodule)
            },
            || Some(std::time::Duration::from_micros(5)),
            std::convert::identity,
        )?;
        Ok(())
    }

    fn analyze_blob<A: Analyzer<P>, P: AsRef<Path>>(
        &self,
        analyzer: A,
        filepath: impl AsRef<Path>,
        repo: &gix::Repository,
        state: &mut GitState,
        result: &mut BlobEntry,
        is_submodule: bool,
    ) -> Result<()> {
        let filepath = filepath.as_ref();
        let blob = repo.find_object(result.index_entry.id)?;
        let contents = blob.data.as_slice();
        state
            .attr_stack
            .at_path(filepath, Some(false), |id, buf| {
                repo.objects.find_blob(id, buf)
            })?
            .matching_attributes(&mut state.attr_matches);

        let mut attrs = [None, None, None, None, None];
        state
            .attr_matches
            .iter_selected()
            .zip(attrs.iter_mut())
            .for_each(|(info, slot)| {
                *slot =
                    (info.assignment.state != gix::attrs::StateRef::Unspecified).then_some(info);
            });

        let lang_override = attrs[0]
            .as_ref()
            .and_then(|info| match info.assignment.state {
                StateRef::Value(v) => v.as_bstr().to_str().ok().map(|s| s.replace('-', " ")),
                _ => None,
            })
            .and_then(|s| self.analyzers.get(&s));

        let language =
            lang_override.or_else(|| self.analyzers.pick(filepath, contents, self.read_limit));

        let language = if let Some(language) = language {
            language
        } else {
            return Ok(());
        };

        // NOTE Unspecified attributes are None, so `state.is_set()` is
        //      implicitly `!state.is_unset()`.
        let generated = attrs[1]
            .as_ref()
            .map(|info| info.assignment.state.is_set())
            .unwrap_or_else(|| self.is_generated(filepath, contents));
        let documentation = attrs[2]
            .as_ref()
            .map(|info| info.assignment.state.is_set())
            .unwrap_or_else(|| self.is_documentation(filepath, contents));
        let vendored = attrs[3]
            .as_ref()
            .map(|info| info.assignment.state.is_set())
            .unwrap_or_else(|| is_submodule || self.is_vendored(filepath, contents));

        let detectable = match language.category() {
            Category::Data | Category::Prose => false,
            Category::Programming | Category::Markup | Category::Query => {
                !(generated || documentation || vendored)
            }
        };
        let detectable = attrs[4]
            .as_ref()
            .map(|info| info.assignment.state.is_set())
            .unwrap_or(detectable);

        let size = contents.len();
        let entry = Entry {
            language: language.clone(),
            size,
            detectable,
            generated,
            documentation,
            vendored,
        };
        result.result = Some(entry);
        Ok(())
    }
}

#[derive(Clone)]
struct GitState {
    attr_stack: gix::worktree::Stack,
    attr_matches: gix::attrs::search::Outcome,
}

impl GitState {
    fn new(repo: &gix::Repository, tree_id: &gix::oid) -> crate::Result<(Self, gix::index::State)> {
        let index = repo.index_from_tree(tree_id)?;
        let attr_stack = repo.attributes_only(
            &index,
            gix::worktree::stack::state::attributes::Source::IdMapping,
        )?;
        let attr_matches = attr_stack.selected_attribute_matches([
            "gengo-language",
            "gengo-generated",
            "gengo-documentation",
            "gengo-vendored",
            "gengo-detectable",
        ]);
        Ok((
            Self {
                attr_stack,
                attr_matches,
            },
            index.into_parts().0,
        ))
    }
}

struct BlobEntry {
    // Just for path and id access
    index_entry: gix::index::Entry,
    result: Option<crate::Entry>,
}

/// The result of analyzing a repository or a single submodule
struct Results {
    /// If this is a submodule, the root is not empty and the full path to where our paths start.
    root: BString,
    entries: Vec<BlobEntry>,
    path_storage: gix::index::PathStorage,
}

impl Results {
    /// Create a data structure that holds index entries as well as our results per entry.
    /// Return a list of paths at which submodules can be found, along with their
    /// commit ids.
    fn from_index(
        root: BString,
        index: gix::index::State,
    ) -> (Self, Vec<(BString, gix::ObjectId)>) {
        use gix::index::entry::Mode;

        let (entries, path_storage) = index.into_entries();
        let submodules: Vec<_> = entries
            .iter()
            .filter(|e| e.mode == Mode::COMMIT)
            .map(|e| (e.path_in(&path_storage).to_owned(), e.id))
            .collect();
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|e| matches!(e.mode, Mode::FILE | Mode::FILE_EXECUTABLE))
            .map(|e| BlobEntry {
                index_entry: e,
                result: None,
            })
            .collect();
        (
            Results {
                root,
                entries,
                path_storage,
            },
            submodules,
        )
    }
}
