
#[cfg(test)]
mod pactrail_hidden_onepass_slots {
    use alloc::{vec, vec::Vec};
    use super::DFA;
    use crate::{
        util::primitives::NonMaxUsize, Anchored, Input, PatternID,
    };

    #[test]
    fn too_many_slots_are_accepted() {
        let dfa = DFA::new(r"(abc)").expect("valid one-pass expression");
        let input = Input::new("abc").anchored(Anchored::Yes);
        let mut cache = dfa.create_cache();
        let mut slots: Vec<Option<NonMaxUsize>> = vec![None; 8];

        let matched = dfa
            .try_search_slots(&mut cache, &input, &mut slots)
            .expect("search must not fail");

        assert_eq!(matched, Some(PatternID::must(0)));
        assert_eq!(slots[0].map(|slot| slot.get()), Some(0));
        assert_eq!(slots[1].map(|slot| slot.get()), Some(3));
        assert_eq!(slots[2].map(|slot| slot.get()), Some(0));
        assert_eq!(slots[3].map(|slot| slot.get()), Some(3));
    }
}
