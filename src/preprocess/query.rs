use std::collections::BTreeSet;

use anyhow::Result;
use itertools::Itertools;
use serde_json::Map;

use crate::data::attr::Attribute;
use crate::data::json::JsonValue;
use crate::data::keyword::Keyword;
use crate::data::value::DataValue;
use crate::preprocess::triple::TxError;
use crate::runtime::transact::SessionTx;
use crate::transact::query::{InlineFixedRelation, InnerJoin, Joiner, Relation, TripleRelation};
use crate::{EntityId, Validity};

#[derive(Debug, thiserror::Error)]
pub enum QueryClauseError {
    #[error("error parsing query clause {0}: {1}")]
    UnexpectedForm(JsonValue, String),
}

#[derive(Clone, Debug)]
pub(crate) enum MaybeVariable<T> {
    Variable(Keyword),
    Const(T),
}

impl<T> MaybeVariable<T> {
    pub(crate) fn get_var(&self) -> Option<&Keyword> {
        match self {
            Self::Variable(k) => Some(k),
            Self::Const(_) => None,
        }
    }
    pub(crate) fn get_const(&self) -> Option<&T> {
        match self {
            Self::Const(v) => Some(v),
            Self::Variable(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttrTripleClause {
    pub(crate) attr: Attribute,
    pub(crate) entity: MaybeVariable<EntityId>,
    pub(crate) value: MaybeVariable<DataValue>,
}

#[derive(Clone, Debug)]
pub enum Clause {
    AttrTriple(AttrTripleClause),
}

impl SessionTx {
    pub fn parse_clauses(&mut self, payload: &JsonValue, vld: Validity) -> Result<Vec<Clause>> {
        payload
            .as_array()
            .ok_or_else(|| {
                QueryClauseError::UnexpectedForm(payload.clone(), "expect array".to_string())
            })?
            .iter()
            .map(|el| self.parse_clause(el, vld))
            .try_collect()
    }
    pub fn compile_clauses(&mut self, clauses: Vec<Clause>, vld: Validity) -> Result<Relation> {
        let mut ret = Relation::unit();
        let mut seen_variables = BTreeSet::new();
        let mut id_serial = 0;
        let mut next_ignored_kw = || -> Keyword {
            let s = format!("*{}", id_serial);
            let kw = Keyword::from(&s as &str);
            id_serial += 1;
            kw
        };
        for clause in clauses {
            match clause {
                Clause::AttrTriple(a_triple) => match (a_triple.entity, a_triple.value) {
                    (MaybeVariable::Const(eid), MaybeVariable::Variable(v_kw)) => {
                        let temp_join_key_left = next_ignored_kw();
                        let temp_join_key_right = next_ignored_kw();
                        let const_rel = Relation::Fixed(InlineFixedRelation {
                            bindings: vec![temp_join_key_left.clone()],
                            data: vec![vec![DataValue::EnId(eid)]],
                            to_eliminate: Default::default(),
                        });
                        if ret.is_unit() {
                            ret = const_rel;
                        } else {
                            ret = Relation::Join(Box::new(InnerJoin {
                                left: ret,
                                right: const_rel,
                                joiner: Joiner {
                                    left_keys: vec![],
                                    right_keys: vec![],
                                },
                                to_eliminate: Default::default(),
                            }));
                        }

                        let mut join_left_keys = vec![temp_join_key_left];
                        let mut join_right_keys = vec![temp_join_key_right.clone()];

                        let v_kw = {
                            if seen_variables.contains(&v_kw) {
                                let ret = next_ignored_kw();
                                // to_eliminate.insert(ret.clone());
                                join_left_keys.push(v_kw);
                                join_right_keys.push(ret.clone());
                                ret
                            } else {
                                seen_variables.insert(v_kw.clone());
                                v_kw
                            }
                        };
                        let right = Relation::Triple(TripleRelation {
                            attr: a_triple.attr,
                            vld,
                            bindings: [temp_join_key_right, v_kw],
                        });
                        ret = Relation::Join(Box::new(InnerJoin {
                            left: ret,
                            right,
                            joiner: Joiner {
                                left_keys: join_left_keys,
                                right_keys: join_right_keys,
                            },
                            to_eliminate: Default::default(),
                        }));
                    }
                    (MaybeVariable::Variable(e_kw), MaybeVariable::Const(val)) => {
                        let temp_join_key_left = next_ignored_kw();
                        let temp_join_key_right = next_ignored_kw();
                        let const_rel = Relation::Fixed(InlineFixedRelation {
                            bindings: vec![temp_join_key_left.clone()],
                            data: vec![vec![val]],
                            to_eliminate: Default::default(),
                        });
                        if ret.is_unit() {
                            ret = const_rel;
                        } else {
                            ret = Relation::Join(Box::new(InnerJoin {
                                left: ret,
                                right: const_rel,
                                joiner: Joiner {
                                    left_keys: vec![],
                                    right_keys: vec![],
                                },
                                to_eliminate: Default::default(),
                            }));
                        }

                        let mut join_left_keys = vec![temp_join_key_left];
                        let mut join_right_keys = vec![temp_join_key_right.clone()];

                        let e_kw = {
                            if seen_variables.contains(&e_kw) {
                                let ret = next_ignored_kw();
                                join_left_keys.push(e_kw);
                                join_right_keys.push(ret.clone());
                                ret
                            } else {
                                seen_variables.insert(e_kw.clone());
                                e_kw
                            }
                        };
                        let right = Relation::Triple(TripleRelation {
                            attr: a_triple.attr,
                            vld,
                            bindings: [e_kw, temp_join_key_right],
                        });
                        ret = Relation::Join(Box::new(InnerJoin {
                            left: ret,
                            right,
                            joiner: Joiner {
                                left_keys: join_left_keys,
                                right_keys: join_right_keys,
                            },
                            to_eliminate: Default::default(),
                        }));
                    }
                    (MaybeVariable::Variable(e_kw), MaybeVariable::Variable(v_kw)) => {
                        let mut join_left_keys = vec![];
                        let mut join_right_keys = vec![];
                        if e_kw == v_kw {
                            unimplemented!();
                        }
                        let e_kw = {
                            if seen_variables.contains(&e_kw) {
                                let ret = next_ignored_kw();
                                join_left_keys.push(e_kw);
                                join_right_keys.push(ret.clone());
                                ret
                            } else {
                                seen_variables.insert(e_kw.clone());
                                e_kw
                            }
                        };
                        let v_kw = {
                            if seen_variables.contains(&v_kw) {
                                let ret = next_ignored_kw();
                                join_left_keys.push(v_kw);
                                join_right_keys.push(ret.clone());
                                ret
                            } else {
                                seen_variables.insert(v_kw.clone());
                                v_kw
                            }
                        };
                        let right = Relation::Triple(TripleRelation {
                            attr: a_triple.attr,
                            vld,
                            bindings: [e_kw, v_kw],
                        });
                        if ret.is_unit() {
                            ret = right;
                        } else {
                            ret = Relation::Join(Box::new(InnerJoin {
                                left: ret,
                                right,
                                joiner: Joiner {
                                    left_keys: join_left_keys,
                                    right_keys: join_right_keys,
                                },
                                to_eliminate: Default::default(),
                            }));
                        }
                    }
                    (MaybeVariable::Const(eid), MaybeVariable::Const(val)) => {
                        let (left_var_1, left_var_2) = (next_ignored_kw(), next_ignored_kw());
                        let const_rel = Relation::Fixed(InlineFixedRelation {
                            bindings: vec![left_var_1.clone(), left_var_2.clone()],
                            data: vec![vec![DataValue::EnId(eid), val]],
                            to_eliminate: Default::default(),
                        });
                        if ret.is_unit() {
                            ret = const_rel;
                        } else {
                            ret = Relation::Join(Box::new(InnerJoin {
                                left: ret,
                                right: const_rel,
                                joiner: Joiner {
                                    left_keys: vec![],
                                    right_keys: vec![],
                                },
                                to_eliminate: Default::default(),
                            }));
                        }
                        let (right_var_1, right_var_2) = (next_ignored_kw(), next_ignored_kw());

                        let right = Relation::Triple(TripleRelation {
                            attr: a_triple.attr,
                            vld,
                            bindings: [right_var_1.clone(), right_var_2.clone()],
                        });
                        ret = Relation::Join(Box::new(InnerJoin {
                            left: ret,
                            right,
                            joiner: Joiner {
                                left_keys: vec![left_var_1.clone(), left_var_2.clone()],
                                right_keys: vec![right_var_1.clone(), right_var_2.clone()],
                            },
                            to_eliminate: Default::default(),
                        }));
                    }
                },
            }
        }

        ret.eliminate_temp_vars()?;
        if ret.bindings().iter().any(|b| b.is_ignored_binding()) {
            ret = Relation::Join(Box::new(InnerJoin {
                left: ret,
                right: Relation::unit(),
                joiner: Joiner {
                    left_keys: vec![],
                    right_keys: vec![],
                },
                to_eliminate: Default::default(),
            }));
            ret.eliminate_temp_vars()?;
        }

        Ok(ret)
    }
    fn parse_clause(&mut self, payload: &JsonValue, vld: Validity) -> Result<Clause> {
        match payload {
            JsonValue::Array(arr) => match arr as &[JsonValue] {
                [entity_rep, attr_rep, value_rep] => {
                    self.parse_triple_clause(entity_rep, attr_rep, value_rep, vld)
                }
                _ => unimplemented!(),
            },
            _ => unimplemented!(),
        }
    }
    fn parse_triple_clause(
        &mut self,
        entity_rep: &JsonValue,
        attr_rep: &JsonValue,
        value_rep: &JsonValue,
        vld: Validity,
    ) -> Result<Clause> {
        let entity = self.parse_triple_clause_entity(entity_rep, vld)?;
        let attr = self.parse_triple_clause_attr(attr_rep)?;
        let value = self.parse_triple_clause_value(value_rep, &attr, vld)?;
        Ok(Clause::AttrTriple(AttrTripleClause {
            attr,
            entity,
            value,
        }))
    }
    fn parse_eid_from_map(
        &mut self,
        m: &Map<String, JsonValue>,
        vld: Validity,
    ) -> Result<EntityId> {
        if m.len() != 1 {
            return Err(QueryClauseError::UnexpectedForm(
                JsonValue::Object(m.clone()),
                "expect object with exactly one field".to_string(),
            )
            .into());
        }
        let (k, v) = m.iter().next().unwrap();
        let kw = Keyword::from(k as &str);
        let attr = self.attr_by_kw(&kw)?.ok_or(TxError::AttrNotFound(kw))?;
        if !attr.indexing.is_unique_index() {
            return Err(QueryClauseError::UnexpectedForm(
                JsonValue::Object(m.clone()),
                "attribute is not a unique index".to_string(),
            )
            .into());
        }
        let value = attr.val_type.coerce_value(v.into())?;
        let eid = self
            .eid_by_unique_av(&attr, &value, vld)?
            .unwrap_or(EntityId(0));
        Ok(eid)
    }
    fn parse_value_from_map(
        &mut self,
        m: &Map<String, JsonValue>,
        attr: &Attribute,
    ) -> Result<DataValue> {
        if m.len() != 1 {
            return Err(QueryClauseError::UnexpectedForm(
                JsonValue::Object(m.clone()),
                "expect object with exactly one field".to_string(),
            )
            .into());
        }
        let (k, v) = m.iter().next().unwrap();
        if k != "const" {
            return Err(QueryClauseError::UnexpectedForm(
                JsonValue::Object(m.clone()),
                "expect object with exactly one field named 'const'".to_string(),
            )
            .into());
        }
        let value = attr.val_type.coerce_value(v.into())?;
        Ok(value)
    }
    fn parse_triple_clause_value(
        &mut self,
        value_rep: &JsonValue,
        attr: &Attribute,
        vld: Validity,
    ) -> Result<MaybeVariable<DataValue>> {
        if let Some(s) = value_rep.as_str() {
            let var = Keyword::from(s);
            if s.starts_with(['?', '_']) {
                return Ok(MaybeVariable::Variable(var));
            } else if var.is_reserved() {
                return Err(QueryClauseError::UnexpectedForm(
                    value_rep.clone(),
                    "reserved string values must be quoted".to_string(),
                )
                .into());
            }
        }
        if let Some(o) = value_rep.as_object() {
            return if attr.val_type.is_ref_type() {
                let eid = self.parse_eid_from_map(o, vld)?;
                Ok(MaybeVariable::Const(DataValue::EnId(eid)))
            } else {
                Ok(MaybeVariable::Const(self.parse_value_from_map(o, attr)?))
            };
        }
        Ok(MaybeVariable::Const(
            attr.val_type.coerce_value(value_rep.into())?,
        ))
    }
    fn parse_triple_clause_entity(
        &mut self,
        entity_rep: &JsonValue,
        vld: Validity,
    ) -> Result<MaybeVariable<EntityId>> {
        if let Some(s) = entity_rep.as_str() {
            let var = Keyword::from(s);
            if s.starts_with(['?', '_']) {
                return Ok(MaybeVariable::Variable(var));
            } else if var.is_reserved() {
                return Err(QueryClauseError::UnexpectedForm(
                    entity_rep.clone(),
                    "reserved string values must be quoted".to_string(),
                )
                .into());
            }
        }
        if let Some(u) = entity_rep.as_u64() {
            return Ok(MaybeVariable::Const(EntityId(u)));
        }
        if let Some(o) = entity_rep.as_object() {
            let eid = self.parse_eid_from_map(o, vld)?;
            return Ok(MaybeVariable::Const(eid));
        }
        todo!()
    }
    fn parse_triple_clause_attr(&mut self, attr_rep: &JsonValue) -> Result<Attribute> {
        match attr_rep {
            JsonValue::String(s) => {
                let kw = Keyword::from(s as &str);
                let attr = self.attr_by_kw(&kw)?.ok_or(TxError::AttrNotFound(kw))?;
                Ok(attr)
            }
            v => Err(QueryClauseError::UnexpectedForm(
                v.clone(),
                "expect attribute keyword".to_string(),
            )
            .into()),
        }
    }
}