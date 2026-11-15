"""Bind SQL bytes from the already hash-admitted QA closure, not runtime templates."""
import hashlib

from common import require


def fullrail_query():
    # load_qa must run first. Its bound-module loader admits these modules only
    # from the selected complete source archive and rejects ambient collisions.
    from decision_observations import CONTRACT, QUERY_ID, SCHEMA, layout, query
    from economic_checkpoint import query as checkpoint_query
    from storage import query_spec
    sql, query_id, schema, tables = query_spec(CONTRACT)
    require(sql == query() == checkpoint_query(source_contract=CONTRACT),
            'QA fullrail query exporters disagree')
    require(query_id == QUERY_ID and schema == SCHEMA and tables == layout()[0],
            'QA fullrail query metadata differs')
    raw = sql.encode('utf-8')
    value = {'sql_contract': CONTRACT, 'query_id': query_id, 'schema': schema,
             'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw), 'table_count': len(tables)}
    return value, raw


def validate_query_binding(source_feed):
    expected, _ = fullrail_query()
    require(source_feed.get('sql_contract') == expected['sql_contract'], 'shared QA SQL contract differs')
    supplied = source_feed.get('sql_query')
    require(type(supplied) is dict and set(supplied) == set(expected)
            and all(type(supplied[k]) is type(v) and supplied[k] == v for k, v in expected.items()),
            'shared QA SQL query bytes/schema/census differ from prepared inputs')
    return expected
