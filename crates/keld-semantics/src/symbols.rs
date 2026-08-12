use crate::{DefId, FunctionId, TypeId};
use keld_source::Span;
use keld_syntax::SyntaxNode;

#[derive(Clone)]
pub(crate) struct ParameterSignature {
    pub name: String,
    pub ty: TypeId,
}

#[derive(Clone)]
pub(crate) enum RetirementSignature {
    Parameter(String, Span),
    Any(DefId),
}

#[derive(Clone)]
pub(crate) struct FunctionSignature<'syntax> {
    pub id: FunctionId,
    pub name: String,
    pub parameters: Vec<ParameterSignature>,
    pub return_type: TypeId,
    pub retirements: Vec<RetirementSignature>,
    pub node: &'syntax SyntaxNode,
}
