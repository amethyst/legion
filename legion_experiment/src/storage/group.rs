use super::{
    archetype::{Archetype, ArchetypeIndex},
    component::{Component, ComponentTypeId},
};
use smallvec::SmallVec;
use std::{collections::HashSet, ops::Index};

pub struct GroupDef {
    components: Vec<ComponentTypeId>,
}

impl GroupDef {
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    pub fn add(&mut self, element: ComponentTypeId) {
        assert!(!self.components.contains(&element));
        self.components.push(element);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubGroup(pub usize);

#[derive(Debug)]
pub struct Group {
    components: SmallVec<[(ComponentTypeId, usize); 5]>,
    archetypes: Vec<ArchetypeIndex>,
}

impl Group {
    fn new<T: IntoIterator<Item = ComponentTypeId>>(components: T) -> Self {
        let components: SmallVec<[(ComponentTypeId, usize); 5]> =
            components.into_iter().map(|type_id| (type_id, 0)).collect();
        let mut seen = HashSet::new();
        for (type_id, _) in &components {
            if seen.contains(type_id) {
                panic!("groups must contain unique components");
            }
            seen.insert(*type_id);
        }
        Self {
            components,
            archetypes: Vec::new(),
        }
    }

    fn matches(&self, components: &[ComponentTypeId]) -> Option<SubGroup> {
        let mut subgroup = None;
        for (i, (type_id, _)) in self.components.iter().enumerate() {
            if !components.contains(type_id) {
                break;
            }

            subgroup = Some(SubGroup(i));
        }

        subgroup
    }

    pub fn exact_match(&self, components: &[ComponentTypeId]) -> Option<SubGroup> {
        let mut subgroup = SubGroup(0);
        let mut count = 0;
        for (i, (type_id, _)) in self.components.iter().enumerate() {
            if !components.contains(type_id) {
                break;
            }

            subgroup = SubGroup(i);
            count += 1;
        }

        if count == components.len() {
            Some(subgroup)
        } else {
            None
        }
    }

    pub fn archetypes(&self) -> &[ArchetypeIndex] {
        &self.archetypes
    }

    pub fn components<'a>(&'a self) -> impl Iterator<Item = ComponentTypeId> + 'a {
        self.components.iter().map(|(c, _)| *c)
    }

    pub fn try_insert(
        &mut self,
        arch_index: ArchetypeIndex,
        archetype: &Archetype,
    ) -> Option<usize> {
        if let Some(SubGroup(subgroup_index)) = self.matches(&archetype.layout().component_types())
        {
            let (_, group_end) = &mut self.components[subgroup_index];
            let index = *group_end;
            self.archetypes.insert(index, arch_index);
            for (_, separator) in &mut self.components[..(subgroup_index + 1)] {
                *separator += 1;
            }
            Some(index)
        } else {
            None
        }
    }
}

impl Index<SubGroup> for Group {
    type Output = [ArchetypeIndex];
    fn index(&self, SubGroup(index): SubGroup) -> &Self::Output {
        let (_, group_separator) = self.components[index];
        &self.archetypes[..group_separator]
    }
}

impl From<GroupDef> for Group {
    fn from(def: GroupDef) -> Self {
        Self::new(def.components)
    }
}

pub trait GroupSource {
    fn to_group() -> Group;
}

macro_rules! group_tuple {
    ($head_ty:ident) => {
        impl_group_tuple!($head_ty);
    };
    ($head_ty:ident, $( $tail_ty:ident ),*) => {
        impl_group_tuple!($head_ty, $( $tail_ty ),*);
        group_tuple!($( $tail_ty ),*);
    };
}

macro_rules! impl_group_tuple {
    ( $( $ty: ident ),* ) => {
        impl<$( $ty: Component ),*> GroupSource for ($( $ty, )*) {
            fn to_group() -> Group {
                let mut group = GroupDef::new();
                $(
                    group.add(ComponentTypeId::of::<$ty>());
                )*
                group.into()
            }
        }
    };
}

group_tuple!(A, B, C, D, E, F, G, H, I);
