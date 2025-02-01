use config::Config;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

use errors::{Context as ErrorContext, Result};
use libs::ahash::{HashSet, HashSetExt};
use libs::num_format::Locale::ar;
use libs::sha2::digest::generic_array::arr;
use libs::tera::{to_value, Context, Tera, Value};
use utils::templates::{check_template_fallbacks, render_template};

use crate::library::Library;
use crate::pagination::Pager;
use crate::ser::{SectionSerMode, SerializingPage, SerializingSection};
use crate::taxonomies::{Taxonomy, TaxonomyTerm};
use crate::Section;

#[derive(Clone, Debug, PartialEq, Eq)]
enum PaginationRoot<'a> {
    Section(&'a Section),
    Taxonomy(&'a Taxonomy, &'a TaxonomyTerm),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaginatorDate<'a> {
    /// All pages in the section/taxonomy
    all_pages: Cow<'a, [PathBuf]>,
    /// Pages split in chunks of `paginate_by`
    pub pagers: Vec<Pager<'a>>,
    /// The thing we are creating the paginator for: section or taxonomy
    root: PaginationRoot<'a>,
    /// whether to reverse before grouping
    paginate_reversed: bool,
    // Those below can be obtained from the root but it would make the code more complex than needed
    pub permalink: String,
    path: String,
    pub paginate_path: String,
    template: String,
    /// Whether this is the index section, we need it for the template name
    is_index: bool,
}

impl<'a> PaginatorDate<'a> {
    // skip empty months removes any months that dont have posts in
    fn datetime_to_pages(section: &'a Section, library: &'a Library, skip_empty_months: bool) -> usize {
        if skip_empty_months {
            let all_pages = library.find_pages_by_path(&section.pages);

            let mut ym_map: HashSet<i32> = HashSet::new();
            for page in all_pages {
                ym_map.insert((page.meta.datetime_tuple.unwrap().0 * 10) +   <u8 as Into<i32>>::into(page.meta.datetime_tuple.unwrap().1));
            }
            ym_map.len()
        }else {
            // todo confirm that section.pages is in date order
            let start_end_page = library.find_pages_by_path(&[section.pages.first().unwrap().clone(), section.pages.last().unwrap().clone()]);
            let year_gap = start_end_page.last().unwrap().meta.datetime_tuple.unwrap().0 - start_end_page.first().unwrap().meta.datetime_tuple.unwrap().0;
            // let month_gap = - start_end_page.first().unwrap().meta.datetime_tuple.unwrap().1;
            let val_i32 =  (((year_gap * 12) +  <u8 as Into<i32>>::into( start_end_page.last().unwrap().meta.datetime_tuple.unwrap().1)) - <u8 as Into<i32>>::into( start_end_page.first().unwrap().meta.datetime_tuple.unwrap().1)).abs();
            let val = val_i32 as usize;
            val
        }
    }
    /// Create a new paginator from a section
    /// It will always at least create one pager (the first) even if there are not enough pages to paginate
    pub fn from_section(section: &'a Section, library: &'a Library) -> PaginatorDate<'a> {
        let mut paginator = PaginatorDate {
            all_pages: Cow::from(&section.pages[..]),
            pagers: Vec::with_capacity(Self::datetime_to_pages(section, library, false)),
            root: PaginationRoot::Section(section),
            paginate_reversed: section.meta.paginate_reversed,
            permalink: section.permalink.clone(),
            path: section.path.clone(),
            paginate_path: section.meta.paginate_path.clone(),
            is_index: section.is_index(),
            template: section.get_template_name().to_string(),
        };

        paginator.fill_pagers(library);
        paginator
    }

    /// Create a new paginator from a taxonomy
    /// It will always at least create one pager (the first) even if there are not enough pages to paginate
    pub fn from_taxonomy(
        taxonomy: &'a Taxonomy,
        item: &'a TaxonomyTerm,
        library: &'a Library,
        tera: &Tera,
        theme: &Option<String>,
    ) -> PaginatorDate<'a> {

        let paginate_by = taxonomy.kind.paginate_by.unwrap();
        // Check for taxon-specific template, or use generic as fallback.
        let specific_template = format!("{}/single.html", taxonomy.kind.name);
        let template = check_template_fallbacks(&specific_template, tera, theme)
            .unwrap_or("taxonomy_single.html");
        let mut paginator = PaginatorDate {
            all_pages: Cow::Borrowed(&item.pages),
            pagers: Vec::with_capacity(item.pages.len() / paginate_by),
            paginate_reversed: false,
            root: PaginationRoot::Taxonomy(taxonomy, item),
            permalink: item.permalink.clone(),
            path: item.path.clone(),
            paginate_path: taxonomy.kind.paginate_path().to_owned(),
            is_index: false,
            template: template.to_string(),
        };

        // taxonomy paginators have no sorting so we won't have to reverse
        paginator.fill_pagers(library);
        paginator
    }

    fn fill_pagers(&mut self, library: &'a Library) {
        // the list of pagers
        let mut pages: Vec<((i32, u8),Vec<SerializingPage>)> = vec![];
        // the pages in the current pagers
        let mut current_page = vec![];
        let mut current_page_month: Option<(i32, u8)> = None;


        for p in &*self.all_pages {
            let page = &library.pages[p];
            if !page.meta.render {
                continue;
            }
            page.meta.datetime_tuple;
            if let Some(current_page_month_inner) = current_page_month {
                if current_page_month_inner.0 == page.meta.datetime_tuple.unwrap().0 &&
                    current_page_month_inner.1 == page.meta.datetime_tuple.unwrap().1 {
                    current_page.push(SerializingPage::new(page, Some(library), false));
                }else {
                    pages.push((current_page_month_inner, current_page));
                    current_page = vec![];
                    current_page.push(SerializingPage::new(page, Some(library), false));
                    current_page_month = Some((page.meta.datetime_tuple.unwrap().0, page.meta.datetime_tuple.unwrap().1 ));

                }

            }else {
                current_page_month = Some((page.meta.datetime_tuple.unwrap().0, page.meta.datetime_tuple.unwrap().1 ));
                current_page.push(SerializingPage::new(page, Some(library), false));
            }
        }

        if !current_page.is_empty() {
            pages.push((current_page_month.unwrap(), current_page));
        }

        let mut pagers = vec![];
        for (index, page) in pages.into_iter().enumerate() {
            // First page has no pagination path
            if index == 0 {
                pagers.push(Pager::new(1, page.1, self.permalink.clone(), self.path.clone(), "".to_owned()));
                continue;
            }

            let page_path_leaf = format!("{}-{:02}", page.0.0, page.0.1);

            let page_path = if self.paginate_path.is_empty() {
                format!("{}-{:02}/", page.0.0, page.0.1)
            } else {
                format!("{}/{}-{:02}/", self.paginate_path, page.0.0, page.0.1)
            };
            let permalink = format!("{}{}", self.permalink, page_path);

            let pager_path = if self.is_index {
                format!("/{}", page_path)
            } else if self.path.ends_with('/') {
                format!("{}{}", self.path, page_path)
            } else {
                format!("{}/{}", self.path, page_path)
            };

            pagers.push(Pager::new(index + 1, page.1, permalink, pager_path, page_path_leaf));
        }

        // We always have the index one at least
        if pagers.is_empty() {
            pagers.push(Pager::new(1, vec![], self.permalink.clone(), self.path.clone(), "".to_owned()));
        }

        self.pagers = pagers;
    }

    pub fn build_paginator_context(&self, current_pager: &Pager) -> HashMap<&str, Value> {
        let mut paginator = HashMap::new();
        // the pager index is 1-indexed so we want a 0-indexed one for indexing there
        let pager_index = current_pager.index - 1;

        // Global variables
        paginator.insert("first", to_value(&self.permalink).unwrap());
        let last_pager = &self.pagers[self.pagers.len() - 1];
        paginator.insert("last", to_value(&last_pager.permalink).unwrap());

        // Variables for this specific page
        if pager_index > 0 {
            let prev_pager = &self.pagers[pager_index - 1];
            paginator.insert("previous", to_value(&prev_pager.permalink).unwrap());
        } else {
            paginator.insert("previous", Value::Null);
        }

        if pager_index < self.pagers.len() - 1 {
            let next_pager = &self.pagers[pager_index + 1];
            paginator.insert("next", to_value(&next_pager.permalink).unwrap());
        } else {
            paginator.insert("next", Value::Null);
        }
        paginator.insert("number_pagers", to_value(self.pagers.len()).unwrap());
        let base_url = if self.paginate_path.is_empty() {
            self.permalink.to_string()
        } else {
            format!("{}{}/", self.permalink, self.paginate_path)
        };
        paginator.insert("base_url", to_value(base_url).unwrap());
        paginator.insert("pages", to_value(&current_pager.pages).unwrap());
        paginator.insert("current_index", to_value(current_pager.index).unwrap());
        paginator.insert("total_pages", to_value(self.all_pages.len()).unwrap());

        paginator
    }

    pub fn render_pager(
        &self,
        pager: &Pager,
        config: &Config,
        tera: &Tera,
        library: &Library,
    ) -> Result<String> {
        let mut context = Context::new();
        match self.root {
            PaginationRoot::Section(s) => {
                context.insert(
                    "section",
                    &SerializingSection::new(s, SectionSerMode::MetadataOnly(library)),
                );
                context.insert("lang", &s.lang);
                context.insert("config", &config.serialize(&s.lang));
            }
            PaginationRoot::Taxonomy(t, item) => {
                context.insert("taxonomy", &t.kind);
                context.insert("term", &item.serialize(library));
                context.insert("lang", &t.lang);
                context.insert("config", &config.serialize(&t.lang));
            }
        };
        context.insert("current_url", &pager.permalink);
        context.insert("current_path", &pager.path);
        context.insert("paginator", &self.build_paginator_context(pager));

        render_template(&self.template, tera, context, &config.theme)
            .with_context(|| format!("Failed to render pager {}", pager.index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Page, SectionFrontMatter, SortBy};

    fn create_section(is_index: bool, paginate_reversed: bool) -> Section {
        let f = SectionFrontMatter {
            paginate_by: Some(2),
            paginate_path: "page".to_string(),
            sort_by: SortBy::Date,
            paginate_reversed,
            ..Default::default()
        };

        let mut s = Section::new("content/_index.md", f, &PathBuf::new());
        if !is_index {
            s.path = "/posts/".to_string();
            s.permalink = "https://vincent.is/posts/".to_string();
            s.file.path = PathBuf::from("posts/_index.md");
            s.file.components = vec!["posts".to_string()];
        } else {
            s.path = "/".into();
            s.file.path = PathBuf::from("_index.md");
            s.permalink = "https://vincent.is/".to_string();
        }
        s
    }

    fn create_library(
        is_index: bool,
        num_pages: usize,
        paginate_reversed: bool,
    ) -> (Section, Library) {
        let mut library = Library::default();

        for i in 0..num_pages {
            let mut page = Page::default();
            let date = format!("2025-{:02}-{:02}", ((i % 12)+1) as u8, ((i/12)+1) as u8);
            page.meta.title = Some(date.clone());
            page.file.path = PathBuf::from(&format!("{}.md", i+1));
            // to get more than 1 page for a month render >12 pages
            // page.meta.datetime_tuple = Some((2025, ((i % 12)+1) as u8, ((i/12)+1) as u8));
            page.meta.date = Some(date.clone());
            // page.
            page.meta.date_to_datetime();
            library.insert_page(page);
        }
        library.sort_section_pages();

        let mut section = create_section(is_index, paginate_reversed);
        section.pages = library.pages.keys().cloned().collect();
        section.pages.sort();
        library.insert_section(section.clone());
        library.sort_section_pages();
        let section = library.sections.values().collect::<Vec<_>>()[0].clone();

        (section, library)
    }

    #[test]
    fn test_can_create_section_paginator() {
        let (section, library) = create_library(false, 13, false);
        let paginator = PaginatorDate::from_section(&section, &library);
        assert_eq!(paginator.pagers.len(), 12); // 13 pages across 12 months 2 on 2025-01

        assert_eq!(paginator.pagers[0].index, 1);
        assert_eq!(paginator.pagers[0].pages.len(), 1); // todo
        // assert_eq!(paginator.pagers[0].t.len(), 1); // todo
        assert_eq!(paginator.pagers[0].pages[0].title.clone().unwrap(), "2025-12-01");
        assert_eq!(paginator.pagers[0].permalink, "https://vincent.is/posts/");
        assert_eq!(paginator.pagers[0].path, "/posts/");

        assert_eq!(paginator.pagers[1].index, 2);
        assert_eq!(paginator.pagers[1].pages.len(), 1);
        assert_eq!(paginator.pagers[1].pages[0].title.clone().unwrap(), "2025-11-01");
        assert_eq!(paginator.pagers[1].permalink, "https://vincent.is/posts/page/2025-11/");
        assert_eq!(paginator.pagers[1].path, "/posts/page/2025-11/");

        assert_eq!(paginator.pagers[2].index, 3);
        assert_eq!(paginator.pagers[2].pages.len(), 1);
        assert_eq!(paginator.pagers[2].pages[0].title.clone().unwrap(), "2025-10-01");
        assert_eq!(paginator.pagers[2].permalink, "https://vincent.is/posts/page/2025-10/");
        assert_eq!(paginator.pagers[2].path, "/posts/page/2025-10/");

        assert_eq!(paginator.pagers[3].index, 4);
        assert_eq!(paginator.pagers[3].pages.len(), 1);
        assert_eq!(paginator.pagers[3].pages[0].title.clone().unwrap(), "2025-09-01");
        assert_eq!(paginator.pagers[3].permalink, "https://vincent.is/posts/page/2025-09/");
        assert_eq!(paginator.pagers[3].path, "/posts/page/2025-09/");

        assert_eq!(paginator.pagers[4].index, 5);
        assert_eq!(paginator.pagers[4].pages.len(), 1);
        assert_eq!(paginator.pagers[4].pages[0].title.clone().unwrap(), "2025-08-01");
        // assert_eq!(paginator.pagers[4]., "2025-08-01");
        assert_eq!(paginator.pagers[4].permalink, "https://vincent.is/posts/page/2025-08/");
        assert_eq!(paginator.pagers[4].path, "/posts/page/2025-08/");



        assert_eq!(paginator.pagers[11].index, 12);
        assert_eq!(paginator.pagers[11].pages.len(), 2);
        //todo is this the right way round?
        assert_eq!(paginator.pagers[11].pages[0].title.clone().unwrap(), "2025-01-02");
        assert_eq!(paginator.pagers[11].pages[1].title.clone().unwrap(), "2025-01-01");
        assert_eq!(paginator.pagers[11].permalink, "https://vincent.is/posts/page/2025-01/");
        assert_eq!(paginator.pagers[11].path, "/posts/page/2025-01/");
    }

/*
    #[test]
    fn can_create_paginator_for_index() {
        let (section, library) = create_library(true, 3, false);
        let paginator = Paginator::from_section(&section, &library);
        assert_eq!(paginator.pagers.len(), 2);

        assert_eq!(paginator.pagers[0].index, 1);
        assert_eq!(paginator.pagers[0].pages.len(), 2);
        assert_eq!(paginator.pagers[0].permalink, "https://vincent.is/");
        assert_eq!(paginator.pagers[0].path, "/");

        assert_eq!(paginator.pagers[1].index, 2);
        assert_eq!(paginator.pagers[1].pages.len(), 1);
        assert_eq!(paginator.pagers[1].permalink, "https://vincent.is/page/2/");
        assert_eq!(paginator.pagers[1].path, "/page/2/");
    }

    #[test]
    fn test_can_build_paginator_context() {
        let (section, library) = create_library(false, 3, false);
        let paginator = Paginator::from_section(&section, &library);
        assert_eq!(paginator.pagers.len(), 2);

        let context = paginator.build_paginator_context(&paginator.pagers[0]);
        assert_eq!(context["paginate_by"], to_value(2).unwrap());
        assert_eq!(context["first"], to_value("https://vincent.is/posts/").unwrap());
        assert_eq!(context["last"], to_value("https://vincent.is/posts/page/2/").unwrap());
        assert_eq!(context["previous"], to_value::<Option<()>>(None).unwrap());
        assert_eq!(context["next"], to_value("https://vincent.is/posts/page/2/").unwrap());
        assert_eq!(context["current_index"], to_value(1).unwrap());
        assert_eq!(context["pages"].as_array().unwrap().len(), 2);

        let context = paginator.build_paginator_context(&paginator.pagers[1]);
        assert_eq!(context["paginate_by"], to_value(2).unwrap());
        assert_eq!(context["first"], to_value("https://vincent.is/posts/").unwrap());
        assert_eq!(context["last"], to_value("https://vincent.is/posts/page/2/").unwrap());
        assert_eq!(context["next"], to_value::<Option<()>>(None).unwrap());
        assert_eq!(context["previous"], to_value("https://vincent.is/posts/").unwrap());
        assert_eq!(context["current_index"], to_value(2).unwrap());
        assert_eq!(context["total_pages"], to_value(3).unwrap());
        assert_eq!(context["pages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_can_create_paginator_for_taxonomy() {
        let (_, library) = create_library(false, 3, false);
        let tera = Tera::default();
        let taxonomy_def = TaxonomyConfig {
            name: "some tags".to_string(),
            paginate_by: Some(2),
            ..TaxonomyConfig::default()
        };
        let taxonomy_item = TaxonomyTerm {
            name: "Something".to_string(),
            slug: "something".to_string(),
            path: "/some-tags/something/".to_string(),
            permalink: "https://vincent.is/some-tags/something/".to_string(),
            pages: library.pages.keys().cloned().collect(),
        };
        let taxonomy = Taxonomy {
            kind: taxonomy_def,
            lang: "en".to_owned(),
            slug: "some-tags".to_string(),
            path: "/some-tags/".to_string(),
            permalink: "https://vincent.is/some-tags/".to_string(),
            items: vec![taxonomy_item.clone()],
        };
        let paginator = Paginator::from_taxonomy(&taxonomy, &taxonomy_item, &library, &tera, &None);
        assert_eq!(paginator.pagers.len(), 2);

        assert_eq!(paginator.pagers[0].index, 1);
        assert_eq!(paginator.pagers[0].pages.len(), 2);
        assert_eq!(paginator.pagers[0].permalink, "https://vincent.is/some-tags/something/");
        assert_eq!(paginator.pagers[0].path, "/some-tags/something/");

        assert_eq!(paginator.pagers[1].index, 2);
        assert_eq!(paginator.pagers[1].pages.len(), 1);
        assert_eq!(paginator.pagers[1].permalink, "https://vincent.is/some-tags/something/page/2/");
        assert_eq!(paginator.pagers[1].path, "/some-tags/something/page/2/");
    }

    // https://github.com/getzola/zola/issues/866
    #[test]
    fn works_with_empty_paginate_path() {
        let (mut section, library) = create_library(false, 3, false);
        section.meta.paginate_path = String::new();
        let paginator = Paginator::from_section(&section, &library);
        assert_eq!(paginator.pagers.len(), 2);

        assert_eq!(paginator.pagers[0].index, 1);
        assert_eq!(paginator.pagers[0].pages.len(), 2);
        assert_eq!(paginator.pagers[0].permalink, "https://vincent.is/posts/");
        assert_eq!(paginator.pagers[0].path, "/posts/");

        assert_eq!(paginator.pagers[1].index, 2);
        assert_eq!(paginator.pagers[1].pages.len(), 1);
        assert_eq!(paginator.pagers[1].permalink, "https://vincent.is/posts/2/");
        assert_eq!(paginator.pagers[1].path, "/posts/2/");

        let context = paginator.build_paginator_context(&paginator.pagers[0]);
        assert_eq!(context["base_url"], to_value("https://vincent.is/posts/").unwrap());
    }*/
}
