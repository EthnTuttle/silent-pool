"""Independent row conservation: gross fees are not a net pool-UTXO floor."""
from oracle import integer, require, unhex


def net_pool_output(revenue,raw,ledger):
    revenue=integer(revenue);fresh={identity:integer(value) for identity,value in raw.items()}
    for identity in fresh:unhex(identity,32)
    gross=revenue-sum(fresh.values());require(gross>=0,"raw rewards exceed block revenue")
    paid=ark=forfeit=delta=0;seen=set()
    for row in ledger:
        identity=bytes(integer(v,0,255) for v in row["sp_id"]).hex();unhex(identity,32)
        require(identity not in seen,"duplicate row in revenue conservation");seen.add(identity)
        p=integer(row["paid_sats"]);a=integer(row["ark_due_sats"]);f=integer(row["forfeited_sats"])
        d=integer(row["pending_delta"],-(1<<63),(1<<63)-1)
        require(p+a+f+d==fresh.get(identity,0),"row paid/Ark/forfeit/delta differs from raw earnings")
        paid+=p;ark+=a;forfeit+=f;delta+=d
    require({i for i,n in fresh.items() if n>0}<=seen,"raw earnings omitted from conservation rows")
    require(paid<=revenue,"sumPaid exceeds actual block revenue")
    net=revenue-paid
    require(net==gross+ark+forfeit+delta,"net pool output differs from gross K plus Ark/forfeit/pending delta")
    return {"revenue":revenue,"raw_rewards":sum(fresh.values()),"gross_K":gross,"sum_paid":paid,
            "ark_due":ark,"forfeit":forfeit,"pending_delta":delta,"net_pool_output":net}
